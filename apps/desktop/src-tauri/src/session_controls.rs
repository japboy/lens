//! Operation-scoped ACP controls. Protocol responders never cross the WebView boundary.
use crate::app_state::{update_lens_state, AppState};
use agent_client_protocol::{schema::v1::*, Agent, ConnectionTo, Error, Responder};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, oneshot, watch};
pub use usecase::session_controls::*;
use uuid::Uuid;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DECISION_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_INTERACTIONS: usize = 32;

enum PermissionAdmission {
    Automatic(RequestPermissionOutcome),
    Pending(Uuid, oneshot::Receiver<InteractionResponse>),
}

struct PendingDecision {
    id: Uuid,
    sender: oneshot::Sender<InteractionResponse>,
}
struct Runtime {
    state: AgentSessionControlState,
    decisions: Vec<PendingDecision>,
    approved_mode: String,
    next_sequence: u32,
    tool_policies: crate::agent_preferences::ToolPolicies,
    active_turn: Option<Uuid>,
    tools: BTreeMap<String, ToolCallUpdateFields>,
}
pub struct SessionControls {
    runtime: Mutex<Runtime>,
    commands: mpsc::Sender<ConfigChange>,
    shutdown: watch::Receiver<bool>,
}

fn invalid(message: &str) -> Error {
    Error::invalid_params().data(message)
}
fn lock_error() -> String {
    "Agent session controls are unavailable".into()
}

impl SessionControls {
    pub fn new(
        operation_id: Uuid,
        session_id: String,
        agent_name: String,
        policy_default: String,
        options: Option<Vec<SessionConfigOption>>,
        modes: Vec<SessionMode>,
        shutdown: watch::Receiver<bool>,
    ) -> Result<(Arc<Self>, mpsc::Receiver<ConfigChange>), Error> {
        if let Some(options) = &options {
            validate_options(options)?;
        }
        let (commands, receiver) = mpsc::channel(1);
        let state = AgentSessionControlState {
            instance_id: Uuid::new_v4(),
            operation_id,
            session_id,
            agent_name,
            active: true,
            config_revision: 0,
            config_options: options,
            modes,
            effective_mode: policy_default.clone(),
            configured_mode: policy_default.clone(),
            configured_origin: ModeOrigin::Policy,
            last_mode_origin: ModeOrigin::Policy,
            policy_default: policy_default.clone(),
            change: None,
            notice: None,
            interactions: Vec::new(),
        };
        Ok((
            Arc::new(Self {
                runtime: Mutex::new(Runtime {
                    state,
                    decisions: Vec::new(),
                    approved_mode: policy_default,
                    next_sequence: 0,
                    tool_policies: Default::default(),
                    active_turn: None,
                    tools: BTreeMap::new(),
                }),
                commands,
                shutdown,
            }),
            receiver,
        ))
    }
    pub fn set_initial_authority(
        &self,
        mode: String,
        policies: crate::agent_preferences::ToolPolicies,
    ) -> Result<(), Error> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| invalid("Agent controls unavailable"))?;
        runtime.approved_mode = mode.clone();
        runtime.state.configured_mode = mode.clone();
        runtime.state.configured_origin = if mode == runtime.state.policy_default {
            ModeOrigin::Policy
        } else {
            ModeOrigin::User
        };
        runtime.state.last_mode_origin = runtime.state.configured_origin;
        runtime.state.effective_mode = mode;
        runtime.tool_policies = policies;
        Ok(())
    }
    pub fn snapshot(&self) -> Result<AgentSessionControlState, String> {
        Ok(self.runtime.lock().map_err(|_| lock_error())?.state.clone())
    }
    pub fn publish<R: tauri::Runtime>(&self, app: &AppHandle<R>) -> Result<(), String> {
        // Serialize mutation/publication so a slower publisher cannot replace a newer snapshot.
        let runtime = self.runtime.lock().map_err(|_| lock_error())?;
        let snapshot = runtime.state.clone();
        update_lens_state(app, snapshot.operation_id, |lens| {
            if lens
                .session_controls
                .as_ref()
                .is_none_or(|s| s.instance_id == snapshot.instance_id)
            {
                lens.session_controls = Some(snapshot);
            }
        })?;
        Ok(())
    }
    fn ensure_active(&self, runtime: &Runtime) -> Result<(), String> {
        if !runtime.state.active || *self.shutdown.borrow() {
            return Err("Agent session has ended".into());
        }
        Ok(())
    }
    pub fn queue_change(
        &self,
        instance_id: Uuid,
        revision: u32,
        config_id: String,
        value: String,
    ) -> Result<(), String> {
        let mut runtime = self.runtime.lock().map_err(|_| lock_error())?;
        self.ensure_active(&runtime)?;
        if runtime.state.instance_id != instance_id || runtime.state.config_revision != revision {
            return Err("Agent settings changed; review the current choices".into());
        }
        if runtime
            .state
            .change
            .as_ref()
            .is_some_and(|c| c.status == ChangeStatus::Pending)
            || !runtime.decisions.is_empty()
        {
            return Err("Resolve the pending Agent interaction first".into());
        }
        Self::validate_change(&runtime.state, &config_id, &value)
            .map_err(|_| "The Agent did not advertise this choice".to_string())?;
        let change = ConfigChange {
            config_id,
            value,
            status: ChangeStatus::Pending,
        };
        self.commands
            .try_send(change.clone())
            .map_err(|_| "Agent settings are busy".to_string())?;
        runtime.state.change = Some(change);
        Ok(())
    }
    fn validate_change(
        state: &AgentSessionControlState,
        id: &str,
        value: &str,
    ) -> Result<bool, Error> {
        if let Some(options) = &state.config_options {
            let option = options
                .iter()
                .find(|o| o.id.to_string() == id)
                .ok_or_else(|| invalid("Unknown Agent selector"))?;
            if !values(option)?.iter().any(|o| o.value.to_string() == value) {
                return Err(invalid("Unknown Agent choice"));
            }
            Ok(option.category == Some(SessionConfigOptionCategory::Mode))
        } else if id == "mode" && state.modes.iter().any(|m| m.id.to_string() == value) {
            Ok(true)
        } else {
            Err(invalid("Unknown Agent selector"))
        }
    }
    pub fn replace_options(&self, options: Vec<SessionConfigOption>) -> Result<(), Error> {
        validate_options(&options)?;
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| invalid("Agent controls unavailable"))?;
        if let Some(mode) = mode_option(&options)? {
            let effective = current_value(mode)?;
            runtime.state.last_mode_origin = ModeOrigin::Agent;
            if effective != runtime.approved_mode {
                return Err(invalid("Agent changed mode without approval"));
            }
            runtime.state.effective_mode = effective;
        } else if runtime
            .state
            .config_options
            .as_ref()
            .is_some_and(|old| mode_option(old).ok().flatten().is_some())
        {
            return Err(invalid("Agent removed its authoritative mode selector"));
        }
        runtime.state.config_revision = runtime
            .state
            .config_revision
            .checked_add(1)
            .ok_or_else(|| invalid("Agent configuration revision exhausted"))?;
        runtime.state.config_options = Some(options);
        Ok(())
    }
    pub fn record_mode(&self, mode: &str) -> Result<(), Error> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| invalid("Agent controls unavailable"))?;
        runtime.state.last_mode_origin = ModeOrigin::Agent;
        if mode != runtime.approved_mode {
            return Err(invalid("Agent changed mode without approval"));
        }
        runtime.state.effective_mode = mode.into();
        Ok(())
    }
    pub fn close<R: tauri::Runtime>(&self, app: &AppHandle<R>) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.state.active = false;
            for interaction in &mut runtime.state.interactions {
                if interaction.status == InteractionStatus::Pending {
                    interaction.status = InteractionStatus::Cancelled;
                }
                interaction.details = None;
            }
            if let Some(change) = &mut runtime.state.change {
                if change.status == ChangeStatus::Pending {
                    change.status = ChangeStatus::Failed;
                }
            }
            for pending in runtime.decisions.drain(..) {
                let _ = pending.sender.send(InteractionResponse::Cancel);
            }
        }
        let _ = self.publish(app);
    }
    fn begin_decision(
        &self,
        details: InteractionDetails,
    ) -> Result<(Uuid, oneshot::Receiver<InteractionResponse>), String> {
        let mut runtime = self.runtime.lock().map_err(|_| lock_error())?;
        self.ensure_active(&runtime)?;
        if runtime.decisions.len() >= 8 {
            return Err("Too many pending Agent interactions".into());
        }
        if runtime.state.interactions.len() == MAX_INTERACTIONS {
            let i = runtime
                .state
                .interactions
                .iter()
                .position(|i| i.status != InteractionStatus::Pending)
                .ok_or_else(|| "Agent interaction history is full".to_string())?;
            runtime.state.interactions.remove(i);
        }
        runtime.next_sequence = runtime
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| "Agent interaction sequence exhausted".to_string())?;
        let sequence = runtime.next_sequence;
        let id = Uuid::new_v4();
        let (sender, receiver) = oneshot::channel();
        let run_id = runtime.active_turn;
        runtime.state.interactions.push(AgentInteraction {
            id,
            run_id,
            sequence,
            status: InteractionStatus::Pending,
            details: Some(details),
        });
        runtime.decisions.push(PendingDecision { id, sender });
        Ok((id, receiver))
    }
    pub fn respond<R: tauri::Runtime>(
        &self,
        app: &AppHandle<R>,
        instance_id: Uuid,
        id: Uuid,
        response: InteractionResponse,
    ) -> Result<(), String> {
        use tauri_plugin_opener::OpenerExt;
        self.respond_with(instance_id, id, response, |url| {
            app.opener()
                .open_url(url.to_owned(), None::<String>)
                .map_err(|_| "Unable to open the elicitation URL".into())
        })
    }
    fn respond_with(
        &self,
        instance_id: Uuid,
        id: Uuid,
        response: InteractionResponse,
        open_url: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut runtime = self.runtime.lock().map_err(|_| lock_error())?;
        self.ensure_active(&runtime)?;
        if runtime.state.instance_id != instance_id {
            return Err("Stale Agent session".into());
        }
        let interaction = runtime
            .state
            .interactions
            .iter()
            .find(|i| i.id == id && i.status == InteractionStatus::Pending)
            .ok_or_else(|| "This interaction has already ended".to_string())?;
        let status = match (&interaction.details, &response) {
            (_, InteractionResponse::Cancel) => InteractionStatus::Cancelled,
            (
                Some(InteractionDetails::Form { schema, .. }),
                InteractionResponse::Submit { content },
            ) => {
                crate::elicitation::validate_content(schema, content)?;
                InteractionStatus::Accepted
            }
            (
                Some(InteractionDetails::Form { .. } | InteractionDetails::Url { .. }),
                InteractionResponse::Decline,
            ) => InteractionStatus::Declined,
            (Some(InteractionDetails::Url { url, .. }), InteractionResponse::Accept) => {
                crate::elicitation::validate_url(url)?;
                open_url(url)?;
                InteractionStatus::Accepted
            }
            (Some(InteractionDetails::ModeTransition { .. }), InteractionResponse::Accept) => {
                InteractionStatus::Accepted
            }
            (Some(InteractionDetails::ModeTransition { .. }), InteractionResponse::Decline) => {
                InteractionStatus::Declined
            }
            (
                Some(InteractionDetails::Permission { options, .. }),
                InteractionResponse::Select { option_id },
            ) => match options
                .iter()
                .find(|o| o.option_id.to_string() == *option_id)
                .map(|o| o.kind)
            {
                Some(PermissionOptionKind::AllowOnce) => InteractionStatus::Accepted,
                Some(PermissionOptionKind::RejectOnce) => InteractionStatus::Declined,
                _ => return Err("Unsupported permission choice".into()),
            },
            _ => return Err("Response does not match the pending interaction".into()),
        };
        let index = runtime
            .decisions
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| "Interaction responder is no longer available".to_string())?;
        let pending = runtime.decisions.remove(index);
        let interaction = runtime
            .state
            .interactions
            .iter_mut()
            .find(|i| i.id == id)
            .unwrap();
        interaction.status = status;
        interaction.details = None;
        pending
            .sender
            .send(response)
            .map_err(|_| "Agent interaction was cancelled".into())
    }
    async fn decision<R: tauri::Runtime>(
        &self,
        app: &AppHandle<R>,
        details: InteractionDetails,
        cancellation: Option<agent_client_protocol::RequestCancellation>,
    ) -> InteractionResponse {
        self.decision_with_deadline(app, details, cancellation, DECISION_TIMEOUT)
            .await
    }
    async fn decision_with_deadline<R: tauri::Runtime>(
        &self,
        app: &AppHandle<R>,
        details: InteractionDetails,
        cancellation: Option<agent_client_protocol::RequestCancellation>,
        timeout: Duration,
    ) -> InteractionResponse {
        let Ok((id, receiver)) = self.begin_decision(details) else {
            return InteractionResponse::Cancel;
        };
        self.await_decision(app, id, receiver, cancellation, timeout)
            .await
    }
    async fn await_decision<R: tauri::Runtime>(
        &self,
        app: &AppHandle<R>,
        id: Uuid,
        mut receiver: oneshot::Receiver<InteractionResponse>,
        cancellation: Option<agent_client_protocol::RequestCancellation>,
        timeout: Duration,
    ) -> InteractionResponse {
        let _guard = DecisionLifetime { controls: self, id };
        let _ = self.publish(app);
        let mut shutdown = self.shutdown.clone();
        let response = tokio::select! {
            biased;
            _ = shutdown.changed() => InteractionResponse::Cancel,
            _ = async { if let Some(cancellation) = &cancellation { cancellation.cancelled().await; } else { std::future::pending::<()>().await; } } => InteractionResponse::Cancel,
            response = &mut receiver => response.unwrap_or(InteractionResponse::Cancel),
            _ = tokio::time::sleep(timeout) => {
                if self.expire_decision(id) { InteractionResponse::Cancel }
                else { receiver.await.unwrap_or(InteractionResponse::Cancel) }

            }
        };
        self.cancel_decision(id);
        let _ = self.publish(app);
        response
    }
    fn permission_details(
        &self,
        request: RequestPermissionRequest,
    ) -> Result<InteractionDetails, String> {
        let Ok(state) = self.snapshot() else {
            return Err(
                "Tool permission denied: its effect, policy, or correlation could not be verified"
                    .into(),
            );
        };
        if !state.active || request.session_id.to_string() != state.session_id {
            return Err(
                "Tool permission denied: its effect, policy, or correlation could not be verified"
                    .into(),
            );
        }
        let fields = {
            let Ok(runtime) = self.runtime.lock() else {
                return Err("Tool permission denied: its effect, policy, or correlation could not be verified".into());
            };
            if runtime.active_turn.is_none()
                || !runtime
                    .tools
                    .contains_key(&request.tool_call.tool_call_id.to_string())
            {
                return Err("Tool permission denied: its effect, policy, or correlation could not be verified".into());
            }
            let mut fields = runtime.tools[&request.tool_call.tool_call_id.to_string()].clone();
            if request.tool_call.fields.kind.is_some()
                && request.tool_call.fields.kind != fields.kind
            {
                return Err("Tool permission denied: conflicting effect descriptions".into());
            }
            if request.tool_call.fields.title.is_some() {
                fields.title = request.tool_call.fields.title.clone();
            }
            if request.tool_call.fields.raw_input.is_some() {
                fields.raw_input = request.tool_call.fields.raw_input.clone();
            }
            fields
        };
        let kind = fields.kind.unwrap_or(ToolKind::Other);
        let Some(title) = fields.title else {
            return Err(
                "Tool permission denied: its effect, policy, or correlation could not be verified"
                    .into(),
            );
        };
        let Some(arguments) = fields.raw_input else {
            return Err(
                "Tool permission denied: its effect, policy, or correlation could not be verified"
                    .into(),
            );
        };
        if title.is_empty()
            || title.len() > 1024
            || !arguments.is_object()
            || request.tool_call.tool_call_id.to_string().is_empty()
            || serde_json::to_vec(&arguments).map_or(true, |v| v.len() > 16 * 1024)
        {
            return Err(
                "Tool permission denied: its effect, policy, or correlation could not be verified"
                    .into(),
            );
        }
        let mut ids = BTreeSet::new();
        if request.options.len() > 16
            || request
                .options
                .iter()
                .any(|o| o.option_id.to_string().is_empty() || o.name.len() > 256)
        {
            return Err("Tool permission denied: invalid options".into());
        }
        if request
            .options
            .iter()
            .any(|o| !ids.insert(o.option_id.to_string()))
        {
            return Err(
                "Tool permission denied: its effect, policy, or correlation could not be verified"
                    .into(),
            );
        }
        let options = request
            .options
            .into_iter()
            .filter(|o| {
                matches!(
                    o.kind,
                    PermissionOptionKind::AllowOnce | PermissionOptionKind::RejectOnce
                )
            })
            .collect::<Vec<_>>();
        if options.is_empty() {
            return Err(
                "Tool permission denied: its effect, policy, or correlation could not be verified"
                    .into(),
            );
        }
        Ok(InteractionDetails::Permission {
            tool_call_id: request.tool_call.tool_call_id.to_string(),
            title,
            effect: serde_json::to_value(kind).unwrap().as_str().unwrap().into(),
            arguments,
            options,
        })
    }
    // These are response rules, not an execution sandbox. Only exact one-shot
    // options may be selected automatically; provider-side persistent grants are never inferred.
    fn automatic_permission_outcome(
        &self,
        details: &InteractionDetails,
    ) -> Result<Option<RequestPermissionOutcome>, String> {
        let InteractionDetails::Permission {
            effect, options, ..
        } = details
        else {
            return Ok(None);
        };
        let kind = serde_json::from_value::<ToolKind>(serde_json::Value::String(effect.clone()))
            .unwrap_or(ToolKind::Other);
        let runtime = self.runtime.lock().map_err(|_| lock_error())?;
        self.ensure_active(&runtime)?;
        let policy = crate::agent_preferences::policy_for_tool(&runtime.tool_policies, kind);
        Ok(permission_response(policy, options))
    }
    pub fn receive_permission<R: tauri::Runtime>(
        self: &Arc<Self>,
        app: &AppHandle<R>,
        request: RequestPermissionRequest,
        responder: Responder<RequestPermissionResponse>,
        connection: &ConnectionTo<Agent>,
    ) -> Result<(), Error> {
        let pending = self.permission_details(request).and_then(|details| {
            if let Some(outcome) = self.automatic_permission_outcome(&details)? {
                return Ok(PermissionAdmission::Automatic(outcome));
            }
            self.begin_decision(details)
                .map(|(id, receiver)| PermissionAdmission::Pending(id, receiver))
        });
        let pending = match pending {
            Ok(PermissionAdmission::Automatic(outcome)) => {
                let outcome = if responder.cancellation().is_cancelled() {
                    RequestPermissionOutcome::Cancelled
                } else {
                    outcome
                };
                return responder.respond(RequestPermissionResponse::new(outcome));
            }
            Ok(PermissionAdmission::Pending(id, receiver)) => Ok((id, receiver)),
            Err(notice) => Err(notice),
        };
        let (id, receiver) = match pending {
            Ok(pending) => pending,
            Err(notice) => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.state.notice = Some(notice);
                }
                let _ = self.publish(app);
                return responder.respond(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Cancelled,
                ));
            }
        };
        let _ = self.publish(app);
        let controls = self.clone();
        let app = app.clone();
        let cancellation = responder.cancellation();
        connection.spawn(async move {
            let response = controls
                .await_decision(&app, id, receiver, Some(cancellation), DECISION_TIMEOUT)
                .await;
            let outcome = match response {
                InteractionResponse::Select { option_id } => {
                    RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))
                }
                _ => RequestPermissionOutcome::Cancelled,
            };
            responder.respond(RequestPermissionResponse::new(outcome))
        })
    }
    pub fn receive_elicitation<R: tauri::Runtime>(
        self: &Arc<Self>,
        app: &AppHandle<R>,
        request: CreateElicitationRequest,
        responder: Responder<CreateElicitationResponse>,
        connection: &ConnectionTo<Agent>,
    ) -> Result<(), Error> {
        let details = match self.elicitation_details(request) {
            Ok(details) => details,
            Err(error) => {
                if let Ok(mut runtime) = self.runtime.lock() {
                    runtime.state.notice = Some("Agent input request denied: unsupported extension, schema, or tool authority".into());
                }
                let _ = self.publish(app);
                return responder.respond_with_error(error);
            }
        };
        let (id, receiver) = self
            .begin_decision(details)
            .map_err(|_| invalid("Agent interaction queue unavailable"))?;
        let _ = self.publish(app);
        let controls = self.clone();
        let app = app.clone();
        let cancellation = responder.cancellation();
        connection.spawn(async move {
            let response = controls
                .await_decision(&app, id, receiver, Some(cancellation), DECISION_TIMEOUT)
                .await;
            let action = match response {
                InteractionResponse::Submit { content } => {
                    let content =
                        serde_json::from_value::<BTreeMap<String, ElicitationContentValue>>(
                            content,
                        )
                        .map_err(|_| invalid("Invalid form response"))?;
                    ElicitationAction::Accept(ElicitationAcceptAction::new().content(content))
                }
                InteractionResponse::Accept => {
                    ElicitationAction::Accept(ElicitationAcceptAction::new())
                }
                InteractionResponse::Decline => ElicitationAction::Decline,
                _ => ElicitationAction::Cancel,
            };
            responder.respond(CreateElicitationResponse::new(action))
        })
    }
    fn elicitation_details(
        &self,
        request: CreateElicitationRequest,
    ) -> Result<InteractionDetails, Error> {
        if request.message.len() > 8192
            || request.meta.as_ref().is_some_and(|meta| !meta.is_empty())
        {
            return Err(invalid(
                "Unsupported elicitation extensions or message size",
            ));
        }
        {
            let runtime = self
                .runtime
                .lock()
                .map_err(|_| invalid("Agent controls unavailable"))?;
            self.ensure_active(&runtime)
                .map_err(|_| invalid("Agent session ended"))?;
            match request.scope() {
                ElicitationScope::Session(scope)
                    if scope.session_id.to_string() == runtime.state.session_id
                        && runtime.active_turn.is_some()
                        && scope
                            .tool_call_id
                            .as_ref()
                            .is_none_or(|id| runtime.tools.contains_key(&id.to_string())) => {}
                _ => return Err(invalid("Unsupported or stale elicitation correlation")),
            }
        }
        let details = match request.mode {
            ElicitationMode::Form(form) => {
                let schema = serde_json::to_value(form.requested_schema)
                    .map_err(|_| invalid("Invalid form schema"))?;
                crate::elicitation::validate_schema(&schema)
                    .map_err(|_| invalid("Unsupported form schema or constraints"))?;
                InteractionDetails::Form {
                    message: request.message,
                    schema,
                }
            }
            ElicitationMode::Url(url) => {
                crate::elicitation::validate_url(&url.url)
                    .map_err(|_| invalid("Unsupported elicitation URL"))?;
                InteractionDetails::Url {
                    message: request.message,
                    elicitation_id: url.elicitation_id.to_string(),
                    url: url.url,
                }
            }
            _ => return Err(invalid("Unsupported elicitation mode")),
        };
        Ok(details)
    }
    pub async fn serve<R: tauri::Runtime>(
        &self,
        app: &AppHandle<R>,
        connection: &ConnectionTo<Agent>,
        mut receiver: mpsc::Receiver<ConfigChange>,
    ) -> Result<(), Error> {
        while let Some(change) = receiver.recv().await {
            let state = self
                .snapshot()
                .map_err(|_| invalid("Agent controls unavailable"))?;
            let is_mode = Self::validate_change(&state, &change.config_id, &change.value)?;
            if is_mode
                && change.value != state.policy_default
                && change.value != state.effective_mode
                && !matches!(
                    self.decision(
                        app,
                        InteractionDetails::ModeTransition {
                            from: state.effective_mode.clone(),
                            to: change.value.clone()
                        },
                        None
                    )
                    .await,
                    InteractionResponse::Accept
                )
            {
                self.finish_change(ChangeStatus::Rejected);
                let _ = self.publish(app);
                continue;
            }
            let current = self
                .snapshot()
                .map_err(|_| invalid("Agent controls unavailable"))?;
            Self::validate_change(&current, &change.config_id, &change.value)?;
            if is_mode {
                let mut runtime = self
                    .runtime
                    .lock()
                    .map_err(|_| invalid("Agent controls unavailable"))?;
                runtime.approved_mode = change.value.clone();
                runtime.state.configured_mode = change.value.clone();
                runtime.state.configured_origin = ModeOrigin::User;
            }
            let request = async {
                if state.config_options.is_some() {
                    let response = connection
                        .send_request(SetSessionConfigOptionRequest::new(
                            state.session_id.clone(),
                            change.config_id.clone(),
                            SessionConfigValueId::new(change.value.clone()),
                        ))
                        .block_task()
                        .await?;
                    confirm_choice(&response.config_options, &change.config_id, &change.value)?;
                    self.replace_options(response.config_options)?;
                } else {
                    connection
                        .send_request(SetSessionModeRequest::new(
                            state.session_id.clone(),
                            change.value.clone(),
                        ))
                        .block_task()
                        .await?;
                    self.record_mode(&change.value)?;
                }
                Ok::<(), Error>(())
            };
            match tokio::time::timeout(REQUEST_TIMEOUT, request).await {
                Ok(Ok(())) => self.finish_change(ChangeStatus::Succeeded),
                _ => {
                    self.finish_change(ChangeStatus::Failed);
                    let _ = self.publish(app);
                    // A missing/failed confirmation leaves resulting authority uncertain.
                    return Err(invalid("Agent settings change failed; restart the session"));
                }
            }
            self.publish(app)
                .map_err(|_| invalid("Unable to publish Agent controls"))?;
        }
        Err(invalid("Agent control channel closed"))
    }
    fn expire_decision(&self, id: Uuid) -> bool {
        let Ok(mut runtime) = self.runtime.lock() else {
            return true;
        };
        let Some(interaction) = runtime
            .state
            .interactions
            .iter_mut()
            .find(|i| i.id == id && i.status == InteractionStatus::Pending)
        else {
            return false;
        };
        interaction.status = InteractionStatus::Expired;
        interaction.details = None;
        runtime.decisions.retain(|p| p.id != id);
        true
    }
    fn cancel_decision(&self, id: Uuid) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.decisions.retain(|p| p.id != id);
            if let Some(interaction) = runtime
                .state
                .interactions
                .iter_mut()
                .find(|i| i.id == id && i.status == InteractionStatus::Pending)
            {
                interaction.status = InteractionStatus::Cancelled;
                interaction.details = None;
            }
        }
    }
    pub fn begin_turn(&self, run_id: Uuid) -> Result<(), Error> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| invalid("Agent controls unavailable"))?;
        runtime.active_turn = Some(run_id);
        runtime.tools.clear();
        Ok(())
    }
    pub fn end_turn(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.active_turn = None;
            runtime.tools.clear();
            for pending in runtime.decisions.drain(..) {
                let _ = pending.sender.send(InteractionResponse::Cancel);
            }
            for interaction in &mut runtime.state.interactions {
                if interaction.status == InteractionStatus::Pending {
                    interaction.status = InteractionStatus::Cancelled;
                    interaction.details = None;
                }
            }
        }
    }
    pub fn record_tool(&self, update: &SessionUpdate) -> Result<(), Error> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| invalid("Agent controls unavailable"))?;
        if runtime.active_turn.is_none() {
            return Ok(());
        }
        match update {
            SessionUpdate::ToolCall(call) => {
                if runtime.tools.len() >= 256
                    || runtime.tools.contains_key(&call.tool_call_id.to_string())
                {
                    return Err(invalid("Invalid or excessive tool call identities"));
                }
                let raw_input = call
                    .raw_input
                    .as_ref()
                    .filter(|value| serde_json::to_vec(value).is_ok_and(|v| v.len() <= 16 * 1024))
                    .cloned();
                let fields = ToolCallUpdateFields::new()
                    .kind(call.kind)
                    .title(call.title.clone())
                    .raw_input(raw_input);
                runtime.tools.insert(call.tool_call_id.to_string(), fields);
            }
            SessionUpdate::ToolCallUpdate(update) => {
                if let Some(fields) = runtime.tools.get_mut(&update.tool_call_id.to_string()) {
                    if update.fields.kind.is_some() {
                        fields.kind = update.fields.kind;
                    }
                    if update.fields.title.is_some() {
                        fields.title = update.fields.title.clone();
                    }
                    if update.fields.raw_input.is_some() {
                        fields.raw_input = update
                            .fields
                            .raw_input
                            .as_ref()
                            .filter(|value| {
                                serde_json::to_vec(value).is_ok_and(|v| v.len() <= 16 * 1024)
                            })
                            .cloned();
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn finish_change(&self, status: ChangeStatus) {
        if let Ok(mut runtime) = self.runtime.lock() {
            if let Some(change) = &mut runtime.state.change {
                change.status = status;
            }
        }
    }
}

pub fn active_controls<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
) -> Result<Arc<SessionControls>, String> {
    let state = app.state::<AppState>();
    let controls = state
        .session_controls
        .lock()
        .map_err(|_| lock_error())?
        .clone()
        .ok_or_else(|| "No active Agent session".to_string())?;
    if controls.snapshot()?.operation_id != operation_id
        || state.lens()?.operation_id != Some(operation_id)
    {
        return Err("Stale Lens operation".into());
    }
    Ok(controls)
}

pub struct ControlLifetime<R: tauri::Runtime> {
    pub app: AppHandle<R>,
    pub controls: Arc<SessionControls>,
}
impl<R: tauri::Runtime> Drop for ControlLifetime<R> {
    fn drop(&mut self) {
        self.controls.close(&self.app);
    }
}

/// Resolve and apply defaults against this Agent's current catalog; never reconstruct choices.
pub async fn apply_defaults(
    connection: &ConnectionTo<Agent>,
    session_id: &SessionId,
    mut options: Option<Vec<SessionConfigOption>>,
    modes: Option<&SessionModeState>,
    defaults: &crate::agent_preferences::AgentDefaults,
    safe_mode: &str,
) -> Result<(Option<Vec<SessionConfigOption>>, String), Error> {
    if defaults.choices.len() > 32 {
        return Err(invalid("Too many saved Agent choices"));
    }
    let mut ids = BTreeSet::new();
    if defaults.choices.iter().any(|c| !ids.insert(&c.config_id)) {
        return Err(invalid("Duplicate saved Agent choices"));
    }
    if let Some(catalog) = &options {
        validate_options(catalog)?;
    }
    let mode_id = options
        .as_deref()
        .map(mode_option)
        .transpose()?
        .flatten()
        .map(|o| o.id.to_string());
    let mode_key = mode_id.clone().unwrap_or_else(|| "mode".into());
    let requested_mode = defaults
        .choices
        .iter()
        .find(|c| c.config_id == mode_key)
        .map(|c| c.value.as_str())
        .unwrap_or(safe_mode)
        .to_string();
    if let Some(config_id) = mode_id {
        let option = options
            .as_ref()
            .unwrap()
            .iter()
            .find(|o| o.id.to_string() == config_id)
            .unwrap();
        if !values(option)?
            .iter()
            .any(|v| v.value.to_string() == requested_mode)
        {
            return Err(invalid("Saved mode is unavailable; review Agent settings"));
        }
        let response = connection
            .send_request(SetSessionConfigOptionRequest::new(
                session_id.clone(),
                config_id,
                SessionConfigValueId::new(requested_mode.clone()),
            ))
            .block_task()
            .await?;
        validate_options(&response.config_options)?;
        options = Some(response.config_options);
    } else {
        if !modes.is_some_and(|m| {
            m.available_modes
                .iter()
                .any(|m| m.id.to_string() == requested_mode)
        }) {
            return Err(invalid("Saved mode is unavailable; review Agent settings"));
        }
        connection
            .send_request(SetSessionModeRequest::new(
                session_id.clone(),
                requested_mode.clone(),
            ))
            .block_task()
            .await?;
    }
    for saved in &defaults.choices {
        if saved.config_id == mode_key {
            continue;
        }
        let option = options
            .as_ref()
            .and_then(|o| o.iter().find(|o| o.id.to_string() == saved.config_id))
            .ok_or_else(|| invalid("Saved Agent option is unavailable; review Agent settings"))?;
        if !values(option)?
            .iter()
            .any(|o| o.value.to_string() == saved.value)
        {
            return Err(invalid(
                "Saved Agent choice is unavailable; review Agent settings",
            ));
        }
        let response = connection
            .send_request(SetSessionConfigOptionRequest::new(
                session_id.clone(),
                saved.config_id.clone(),
                SessionConfigValueId::new(saved.value.clone()),
            ))
            .block_task()
            .await?;
        validate_options(&response.config_options)?;
        confirm_choice(&response.config_options, &saved.config_id, &saved.value)?;
        options = Some(response.config_options);
    }
    if let Some(mode) = options.as_deref().map(mode_option).transpose()?.flatten() {
        if current_value(mode)? != requested_mode {
            return Err(invalid("Agent did not confirm the configured mode"));
        }
    }
    if let Some(catalog) = &options {
        for saved in &defaults.choices {
            if saved.config_id != mode_key {
                confirm_choice(catalog, &saved.config_id, &saved.value)?;
            }
        }
    }
    Ok((options, requested_mode))
}

struct DecisionLifetime<'a> {
    controls: &'a SessionControls,
    id: Uuid,
}
impl Drop for DecisionLifetime<'_> {
    fn drop(&mut self) {
        self.controls.cancel_decision(self.id);
    }
}
pub struct TurnLifetime<'a, R: tauri::Runtime> {
    pub controls: &'a SessionControls,
    pub app: &'a AppHandle<R>,
}
impl<R: tauri::Runtime> Drop for TurnLifetime<'_, R> {
    fn drop(&mut self) {
        self.controls.end_turn();
        let _ = self.controls.publish(self.app);
    }
}

/// Synchronously revoke UI responders before the application or operation is torn down.
pub fn close_active<R: tauri::Runtime>(app: &AppHandle<R>) {
    let controls = app
        .state::<AppState>()
        .session_controls
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    if let Some(controls) = controls {
        controls.close(app);
    }
}

#[cfg(debug_assertions)]
#[path = "session_controls_validation.rs"]
pub mod validation;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_preferences::{AgentDefaults, SavedChoice, ToolPolicies};
    use agent_client_protocol::{Client, Responder};

    fn options() -> Vec<SessionConfigOption> {
        vec![
            SessionConfigOption::select(
                "session-mode",
                "Mode",
                "safe",
                vec![
                    SessionConfigSelectOption::new("safe", "Safe"),
                    SessionConfigSelectOption::new("write", "Write"),
                ],
            )
            .category(SessionConfigOptionCategory::Mode),
            SessionConfigOption::select(
                "z-model",
                "Model",
                "first",
                vec![
                    SessionConfigSelectOption::new("first", "First"),
                    SessionConfigSelectOption::new("second", "Second"),
                ],
            )
            .category(SessionConfigOptionCategory::Model),
        ]
    }
    fn controls() -> (Arc<SessionControls>, watch::Sender<bool>) {
        let (shutdown, receiver) = watch::channel(false);
        let (controls, _) = SessionControls::new(
            Uuid::new_v4(),
            "session".into(),
            "Synthetic Agent".into(),
            "safe".into(),
            Some(options()),
            vec![],
            receiver,
        )
        .unwrap();
        (controls, shutdown)
    }
    fn mode_decision() -> InteractionDetails {
        InteractionDetails::ModeTransition {
            from: "safe".into(),
            to: "write".into(),
        }
    }

    #[test]
    fn elicitation_requires_correlation_and_never_grants_unsupported_persistence() {
        let (controls, _shutdown) = controls();
        controls.begin_turn(Uuid::new_v4()).unwrap();
        let form = serde_json::json!({"sessionId":"session","mode":"form","message":"Fixture","requestedSchema":{"type":"object","properties":{}}});
        assert!(controls
            .elicitation_details(serde_json::from_value(form.clone()).unwrap())
            .is_ok());
        let mut extension = form.clone();
        extension["_meta"] =
            serde_json::json!({"codex_approval_kind":"mcp_tool_call","persist":["always"]});
        assert!(controls
            .elicitation_details(serde_json::from_value(extension).unwrap())
            .is_err());
        controls
            .record_tool(&SessionUpdate::ToolCall(
                ToolCall::new("mcp", "Unclassified MCP tool").kind(ToolKind::Other),
            ))
            .unwrap();
        let mut correlated = form;
        correlated["toolCallId"] = "mcp".into();
        assert!(controls
            .elicitation_details(serde_json::from_value(correlated).unwrap())
            .is_ok());
        assert!(controls.snapshot().unwrap().interactions.is_empty());
    }
    #[test]
    fn saved_response_rules_select_only_unambiguous_one_shot_options() {
        use crate::agent_preferences::ToolPolicy;
        let (controls, _shutdown) = controls();
        let mut details = InteractionDetails::Permission {
            tool_call_id: "tool".into(),
            title: "Read".into(),
            effect: "read".into(),
            arguments: serde_json::json!({}),
            options: vec![
                PermissionOption::new("exact-allow", "Allow", PermissionOptionKind::AllowOnce),
                PermissionOption::new("exact-reject", "Reject", PermissionOptionKind::RejectOnce),
            ],
        };
        for (policy, expected) in [
            (ToolPolicy::Ask, None),
            (ToolPolicy::Allow, Some("exact-allow")),
            (ToolPolicy::Deny, Some("exact-reject")),
        ] {
            controls.runtime.lock().unwrap().tool_policies.read = policy;
            let outcome = controls.automatic_permission_outcome(&details).unwrap();
            match expected {
                Some(id) => assert_eq!(
                    serde_json::to_value(outcome.unwrap()).unwrap()["optionId"],
                    id
                ),
                None => assert!(outcome.is_none()),
            }
        }
        controls.runtime.lock().unwrap().tool_policies.read = ToolPolicy::Allow;
        if let InteractionDetails::Permission { effect, .. } = &mut details {
            *effect = "other".into();
        }
        assert!(controls
            .automatic_permission_outcome(&details)
            .unwrap()
            .is_none());
        if let InteractionDetails::Permission {
            effect, options, ..
        } = &mut details
        {
            *effect = "read".into();
            options.push(PermissionOption::new(
                "second-allow",
                "Allow",
                PermissionOptionKind::AllowOnce,
            ));
        }
        assert!(controls
            .automatic_permission_outcome(&details)
            .unwrap()
            .is_none());
        if let InteractionDetails::Permission { options, .. } = &mut details {
            *options = vec![PermissionOption::new(
                "permanent",
                "Always",
                PermissionOptionKind::AllowAlways,
            )];
        }
        assert!(controls
            .automatic_permission_outcome(&details)
            .unwrap()
            .is_none());
        controls.runtime.lock().unwrap().tool_policies.read = ToolPolicy::Deny;
        assert!(matches!(
            controls.automatic_permission_outcome(&details).unwrap(),
            Some(RequestPermissionOutcome::Cancelled)
        ));
        assert!(controls.snapshot().unwrap().interactions.is_empty());
    }

    #[test]
    fn mode_authority_distinguishes_policy_user_and_agent_state() {
        let (controls, _shutdown) = controls();
        assert_eq!(
            controls.snapshot().unwrap().configured_origin,
            ModeOrigin::Policy
        );
        controls
            .set_initial_authority("write".into(), ToolPolicies::default())
            .unwrap();
        assert_eq!(
            controls.snapshot().unwrap().configured_origin,
            ModeOrigin::User
        );
        controls.record_mode("write").unwrap();
        let state = controls.snapshot().unwrap();
        assert_eq!(state.configured_mode, "write");
        assert_eq!(state.effective_mode, "write");
        assert_eq!(state.last_mode_origin, ModeOrigin::Agent);
        assert!(controls.record_mode("unapproved").is_err());
        assert_eq!(controls.snapshot().unwrap().effective_mode, "write");
    }
    #[test]
    fn malformed_or_ambiguous_options_fail_closed() {
        let good = options();
        validate_options(&good).unwrap();
        let mut duplicate = good.clone();
        duplicate.push(good[0].clone());
        assert!(validate_options(&duplicate).is_err());
        let unknown = vec![SessionConfigOption::select(
            "option",
            "Option",
            "missing",
            vec![SessionConfigSelectOption::new("present", "Present")],
        )];
        assert!(validate_options(&unknown).is_err());
        let mut ambiguous = good.clone();
        let mut mode = good[0].clone();
        mode.id = "another-mode".into();
        ambiguous.push(mode);
        assert!(validate_options(&ambiguous).is_err());
    }
    #[test]
    fn full_option_replacement_and_mode_authority_are_independent() {
        let (controls, _shutdown) = controls();
        controls
            .replace_options(vec![options()[0].clone()])
            .unwrap();
        assert_eq!(
            controls.snapshot().unwrap().config_options.unwrap().len(),
            1
        );
        assert!(controls.record_mode("write").is_err());
        assert_eq!(controls.snapshot().unwrap().effective_mode, "safe");
        controls
            .set_initial_authority("write".into(), ToolPolicies::default())
            .unwrap();
        controls.record_mode("write").unwrap();
        assert!(controls.record_mode("unknown").is_err());
        assert!(controls.replace_options(vec![]).is_err());
    }
    #[tokio::test]
    async fn decisions_are_single_use_and_terminal_payloads_are_erased() {
        let (controls, _shutdown) = controls();
        let (first, response) = controls.begin_decision(mode_decision()).unwrap();
        let (second, _) = controls.begin_decision(mode_decision()).unwrap();
        let state = controls.snapshot().unwrap();
        assert_eq!(
            state
                .interactions
                .iter()
                .map(|i| i.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(controls
            .respond_with(Uuid::new_v4(), first, InteractionResponse::Accept, |_| Ok(
                ()
            ))
            .is_err());
        controls
            .respond_with(
                state.instance_id,
                first,
                InteractionResponse::Decline,
                |_| Ok(()),
            )
            .unwrap();
        assert!(matches!(
            response.await.unwrap(),
            InteractionResponse::Decline
        ));
        assert!(controls
            .respond_with(
                state.instance_id,
                first,
                InteractionResponse::Accept,
                |_| Ok(())
            )
            .is_err());
        controls.cancel_decision(second);
        let state = controls.snapshot().unwrap();
        assert_eq!(state.interactions[0].status, InteractionStatus::Declined);
        assert_eq!(state.interactions[1].status, InteractionStatus::Cancelled);
        assert!(state.interactions.iter().all(|i| i.details.is_none()));
    }
    #[test]
    fn stale_revisions_unknown_values_and_closed_sessions_reject_changes() {
        let (controls, shutdown) = controls();
        let state = controls.snapshot().unwrap();
        assert!(controls
            .queue_change(state.instance_id, 99, "z-model".into(), "second".into())
            .is_err());
        assert!(controls
            .queue_change(state.instance_id, 0, "z-model".into(), "invented".into())
            .is_err());
        shutdown.send(true).unwrap();
        assert!(controls.begin_decision(mode_decision()).is_err());
    }
    #[tokio::test]
    async fn ending_a_turn_cancels_pending_responses_and_tool_correlations() {
        let (controls, _shutdown) = controls();
        controls.begin_turn(Uuid::new_v4()).unwrap();
        controls
            .record_tool(&SessionUpdate::ToolCall(
                ToolCall::new("tool", "Read").kind(ToolKind::Read),
            ))
            .unwrap();
        let (id, receiver) = controls.begin_decision(mode_decision()).unwrap();
        controls.end_turn();
        assert!(matches!(
            receiver.await.unwrap(),
            InteractionResponse::Cancel
        ));
        assert!(controls.runtime.lock().unwrap().tools.is_empty());
        assert_eq!(
            controls
                .snapshot()
                .unwrap()
                .interactions
                .iter()
                .find(|i| i.id == id)
                .unwrap()
                .status,
            InteractionStatus::Cancelled
        );
    }
    #[tokio::test]
    async fn form_validation_happens_before_consuming_the_responder() {
        let (controls, _shutdown) = controls();
        let schema = serde_json::json!({"type":"object","properties":{"count":{"type":"integer","minimum":1,"maximum":3}},"required":["count"]});
        let (id, receiver) = controls
            .begin_decision(InteractionDetails::Form {
                message: "Count".into(),
                schema,
            })
            .unwrap();
        let state = controls.snapshot().unwrap();
        assert!(controls
            .respond_with(
                state.instance_id,
                id,
                InteractionResponse::Submit {
                    content: serde_json::json!({"count":4})
                },
                |_| Ok(())
            )
            .is_err());
        assert_eq!(
            controls.snapshot().unwrap().interactions[0].status,
            InteractionStatus::Pending
        );
        controls
            .respond_with(
                state.instance_id,
                id,
                InteractionResponse::Submit {
                    content: serde_json::json!({"count":2}),
                },
                |_| Ok(()),
            )
            .unwrap();
        assert!(matches!(
            receiver.await.unwrap(),
            InteractionResponse::Submit { .. }
        ));
    }
    #[test]
    fn url_opening_requires_the_matching_pending_user_acceptance() {
        let (controls, _shutdown) = controls();
        let (id, _receiver) = controls
            .begin_decision(InteractionDetails::Url {
                message: "Sign in".into(),
                elicitation_id: "url-1".into(),
                url: "https://example.com/consent".into(),
            })
            .unwrap();
        let state = controls.snapshot().unwrap();
        let opened = std::cell::Cell::new(0);
        assert!(controls
            .respond_with(Uuid::new_v4(), id, InteractionResponse::Accept, |_| {
                opened.set(1);
                Ok(())
            })
            .is_err());
        assert_eq!(opened.get(), 0);
        controls
            .respond_with(state.instance_id, id, InteractionResponse::Accept, |url| {
                assert_eq!(url, "https://example.com/consent");
                opened.set(1);
                Ok(())
            })
            .unwrap();
        assert_eq!(opened.get(), 1);
        assert!(controls
            .respond_with(
                state.instance_id,
                id,
                InteractionResponse::Accept,
                |_| panic!("must not open twice")
            )
            .is_err());
    }
    #[test]
    fn permission_requires_current_correlation_without_reinterpreting_agent_mode() {
        let (controls, _shutdown) = controls();
        let request = |kind| {
            RequestPermissionRequest::new(
                "session",
                ToolCallUpdate::new("tool", ToolCallUpdateFields::new().kind(kind)),
                vec![PermissionOption::new(
                    "once",
                    "Allow once",
                    PermissionOptionKind::AllowOnce,
                )],
            )
        };
        assert!(controls
            .permission_details(request(ToolKind::Read))
            .is_err());
        controls.begin_turn(Uuid::new_v4()).unwrap();
        controls
            .record_tool(&SessionUpdate::ToolCall(
                ToolCall::new("tool", "Read file")
                    .kind(ToolKind::Read)
                    .raw_input(serde_json::json!({"path":"fixture"})),
            ))
            .unwrap();
        assert!(controls.permission_details(request(ToolKind::Read)).is_ok());
        assert!(controls
            .permission_details(request(ToolKind::Execute))
            .is_err());
        controls.runtime.lock().unwrap().tool_policies.read =
            crate::agent_preferences::ToolPolicy::Deny;
        assert!(controls.permission_details(request(ToolKind::Read)).is_ok());
        controls.end_turn();
        controls.begin_turn(Uuid::new_v4()).unwrap();
        controls
            .record_tool(&SessionUpdate::ToolCall(
                ToolCall::new("tool", "Write file")
                    .kind(ToolKind::Edit)
                    .raw_input(serde_json::json!({"path":"fixture"})),
            ))
            .unwrap();
        controls.runtime.lock().unwrap().tool_policies.edit =
            crate::agent_preferences::ToolPolicy::Ask;
        assert!(controls.permission_details(request(ToolKind::Edit)).is_ok());
        controls
            .set_initial_authority(
                "write".into(),
                ToolPolicies {
                    edit: crate::agent_preferences::ToolPolicy::Ask,
                    ..ToolPolicies::default()
                },
            )
            .unwrap();
        controls.record_mode("write").unwrap();
        assert!(controls.permission_details(request(ToolKind::Edit)).is_ok());
        controls.end_turn();
        assert!(controls
            .permission_details(request(ToolKind::Edit))
            .is_err());
    }
    #[tokio::test]
    async fn expiry_cannot_overwrite_an_accepted_response() {
        let (controls, _shutdown) = controls();
        let (id, response) = controls.begin_decision(mode_decision()).unwrap();
        controls
            .respond_with(
                controls.snapshot().unwrap().instance_id,
                id,
                InteractionResponse::Accept,
                |_| Ok(()),
            )
            .unwrap();
        assert!(!controls.expire_decision(id));
        assert!(matches!(
            response.await.unwrap(),
            InteractionResponse::Accept
        ));
        let (expired, response) = controls.begin_decision(mode_decision()).unwrap();
        assert!(controls.expire_decision(expired));
        assert!(response.await.is_err());
        assert!(controls
            .snapshot()
            .unwrap()
            .interactions
            .iter()
            .all(|i| i.details.is_none()));
    }
    #[tokio::test]
    async fn saved_defaults_use_exact_config_ids_and_full_dependent_responses() {
        let catalog = options();
        let expected = catalog.clone();
        let agent = Agent.builder().on_receive_request(
            async move |request: SetSessionConfigOptionRequest,
                        responder: Responder<SetSessionConfigOptionResponse>,
                        _connection: ConnectionTo<Client>| {
                assert_eq!(request.session_id.to_string(), "fixture-session");
                assert_eq!(request.config_id.to_string(), "session-mode");
                assert_eq!(request.value.as_value_id().unwrap().to_string(), "safe");
                responder.respond(SetSessionConfigOptionResponse::new(catalog.clone()))
            },
            agent_client_protocol::on_receive_request!(),
        );
        let client = Client
            .builder()
            .connect_with(agent, async move |connection| {
                let defaults = AgentDefaults {
                    choices: vec![SavedChoice {
                        config_id: "session-mode".into(),
                        value: "safe".into(),
                    }],
                    ..AgentDefaults::default()
                };
                let (result, mode) = apply_defaults(
                    &connection,
                    &SessionId::new("fixture-session"),
                    Some(expected.clone()),
                    None,
                    &defaults,
                    "safe",
                )
                .await?;
                assert_eq!(result, Some(expected));
                assert_eq!(mode, "safe");
                Ok(())
            });
        tokio::time::timeout(Duration::from_secs(5), client)
            .await
            .unwrap()
            .unwrap();
    }
}
