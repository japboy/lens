use crate::{
    lens::{LensInput, LensMediaPayload, LensTargetSet},
    live_sync::{LensAgentProjection, ProjectionRef},
    model::{
        AgentRuntimeState, AgentSelectionState, AppConfig, AppSnapshot, LensMonitoringLifecycle,
        LensStage, LensState,
    },
    store::ConfigStore,
};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex, RwLock,
};
use tauri::{AppHandle, Manager};
use tokio::sync::{oneshot, watch, Mutex as AsyncMutex, Notify};
pub use usecase::state::{
    advance_revision, next_revision, AgentRunKey, LensContextRefreshCommit,
    LensContextRefreshOutcome,
};
use usecase::state::{
    agent_run_has_authority, coalesce_agent_turn, prepare_context_refresh, validate_context_state,
    validate_refresh_identity, validated_payload_map,
};
pub(crate) use usecase::state::{AgentSessionIdentity, AgentSessionTurnCompletion};
use uuid::Uuid;

pub struct AgentRunHandle {
    pub key: AgentRunKey,
    pub cancellation: watch::Receiver<bool>,
}

struct ActiveAgentRun {
    key: AgentRunKey,
    cancellation: watch::Sender<bool>,
}

pub(crate) struct AgentSessionTurn {
    pub context_revision: u64,
    pub projection_ref: ProjectionRef,
    pub projection: LensAgentProjection,
    pub app_prompt: Option<Vec<agent_client_protocol::schema::v1::ContentBlock>>,
    completion: Option<oneshot::Sender<Result<AgentSessionTurnCompletion, String>>>,
    work: Option<AgentWorkLease>,
}

impl AgentSessionTurn {
    pub fn new(
        context_revision: u64,
        projection_ref: ProjectionRef,
        projection: LensAgentProjection,
    ) -> (
        Self,
        oneshot::Receiver<Result<AgentSessionTurnCompletion, String>>,
    ) {
        let (completion, receiver) = oneshot::channel();
        (
            Self {
                context_revision,
                projection_ref,
                projection,
                app_prompt: None,
                completion: Some(completion),
                work: None,
            },
            receiver,
        )
    }

    pub(crate) fn track_work(&mut self, count: Arc<AtomicUsize>) {
        count.fetch_add(1, Ordering::AcqRel);
        self.work = Some(AgentWorkLease(count));
    }

    pub fn complete(mut self, result: Result<AgentSessionTurnCompletion, String>) {
        if let Some(completion) = self.completion.take() {
            let _ = completion.send(result);
        }
    }
}

impl Drop for AgentSessionTurn {
    fn drop(&mut self) {
        if let Some(completion) = self.completion.take() {
            let _ = completion.send(Err("Agent session turn ended without a result".into()));
        }
    }
}

pub(crate) struct AgentSessionMailbox {
    pending: Mutex<Option<AgentSessionTurn>>,
    notify: Notify,
    closed: AtomicBool,
}

impl AgentSessionMailbox {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(None),
            notify: Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    pub fn replace(&self, turn: AgentSessionTurn) -> Result<(), String> {
        let previous = {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| "Agent session mailbox lock is poisoned".to_string())?;
            if pending.as_ref().is_some_and(|t| t.app_prompt.is_some()) {
                return Err(
                    "A user App message is already admitted; refresh will retry later".into(),
                );
            }
            coalesce_agent_turn(&mut pending, turn, self.closed.load(Ordering::Acquire))
        }
        .map_err(|_| "Agent session mailbox is closed".to_string())?;
        if let Some(previous) = previous {
            previous.complete(Ok(AgentSessionTurnCompletion::Coalesced));
        }
        self.notify.notify_one();
        Ok(())
    }

    pub fn take_pending(&self) -> Result<Option<AgentSessionTurn>, String> {
        self.pending
            .lock()
            .map(|mut pending| pending.take())
            .map_err(|_| "Agent session mailbox lock is poisoned".to_string())
    }

    pub async fn notified(&self) {
        self.notify.notified().await;
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub fn has_app_message(&self) -> bool {
        self.pending
            .lock()
            .ok()
            .is_some_and(|p| p.as_ref().is_some_and(|t| t.app_prompt.is_some()))
    }

    fn admit_app(&self, turn: AgentSessionTurn) -> Result<(), String> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "Agent mailbox unavailable")?;
        if self.is_closed() || pending.is_some() {
            return Err("Agent is busy; retry after the current turn".into());
        }
        *pending = Some(turn);
        self.notify.notify_one();
        Ok(())
    }

    pub fn close(&self, reason: &str) -> Result<(), String> {
        self.closed.store(true, Ordering::Release);
        if let Some(pending) = self.take_pending()? {
            pending.complete(Err(reason.into()));
        }
        self.notify.notify_waiters();
        Ok(())
    }
}

/// Tracks admitted turns through queueing, startup, execution and cancellation.
/// Ownership follows the turn, including the gap between dequeue and run admission.
struct AgentWorkLease(Arc<AtomicUsize>);

impl Drop for AgentWorkLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct ActiveAgentSession {
    generation: Uuid,
    identity: AgentSessionIdentity,
    mailbox: Arc<AgentSessionMailbox>,
    shutdown: watch::Sender<bool>,
}

#[derive(Default)]
pub struct AgentControl {
    unfinished_turns: Arc<AtomicUsize>,
    active: Mutex<Option<ActiveAgentRun>>,
    session: Mutex<Option<ActiveAgentSession>>,
}

/// Clears only this run when its owning future completes or is dropped.
pub(crate) struct AgentRunLifetime<'a> {
    control: &'a AgentControl,
    key: AgentRunKey,
}

impl Drop for AgentRunLifetime<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.control.finish(self.key) {
            eprintln!("Unable to finish the dropped Agent run: {error}");
        }
    }
}

impl AgentControl {
    pub(crate) fn run_lifetime(&self, key: AgentRunKey) -> AgentRunLifetime<'_> {
        AgentRunLifetime { control: self, key }
    }

    pub(crate) fn has_pending_work(&self) -> Result<bool, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "Agent run state is unavailable".to_string())?;
        Ok(active.is_some() || self.unfinished_turns.load(Ordering::Acquire) != 0)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn begin_validation(&self, operation_id: Uuid) -> Result<AgentRunHandle, String> {
        self.begin(operation_id)
    }

    fn begin(&self, operation_id: Uuid) -> Result<AgentRunHandle, String> {
        let key = AgentRunKey {
            operation_id,
            run_id: Uuid::new_v4(),
        };
        let (cancellation, receiver) = watch::channel(false);
        let mut active = self
            .active
            .lock()
            .map_err(|_| "agent control lock is poisoned".to_string())?;
        if let Some(previous) = active.replace(ActiveAgentRun { key, cancellation }) {
            let _ = previous.cancellation.send(true);
        }
        Ok(AgentRunHandle {
            key,
            cancellation: receiver,
        })
    }

    pub fn cancel_active(&self) -> Result<Option<AgentRunKey>, String> {
        let key = {
            let active = self
                .active
                .lock()
                .map_err(|_| "agent control lock is poisoned".to_string())?;
            active.as_ref().map(|active| {
                let _ = active.cancellation.send(true);
                active.key
            })
        };
        self.shutdown_session(None, "Agent session was cancelled")?;
        Ok(key)
    }

    pub fn cancel(&self, key: AgentRunKey) -> Result<bool, String> {
        let cancelled = {
            let active = self
                .active
                .lock()
                .map_err(|_| "agent control lock is poisoned".to_string())?;
            let Some(active) = active.as_ref().filter(|active| active.key == key) else {
                return Ok(false);
            };
            let _ = active.cancellation.send(true);
            true
        };
        self.shutdown_session(
            Some(key.operation_id),
            "Agent session was cancelled by the user",
        )?;
        Ok(cancelled)
    }

    pub fn finish(&self, key: AgentRunKey) -> Result<bool, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "agent control lock is poisoned".to_string())?;
        if active.as_ref().is_some_and(|run| run.key == key) {
            *active = None;
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn submit_session<R: tauri::Runtime>(
        &self,
        app: AppHandle<R>,
        identity: AgentSessionIdentity,
        context_revision: u64,
        projection_ref: ProjectionRef,
        projection: LensAgentProjection,
    ) -> Result<oneshot::Receiver<Result<AgentSessionTurnCompletion, String>>, String> {
        let (mut turn, receiver) =
            AgentSessionTurn::new(context_revision, projection_ref, projection);
        turn.track_work(Arc::clone(&self.unfinished_turns));
        let (previous, spawn) = {
            let mut session = self
                .session
                .lock()
                .map_err(|_| "Agent session control lock is poisoned".to_string())?;
            if let Some(active) = session.as_ref() {
                if active
                    .identity
                    .admits_reuse(&identity, active.mailbox.is_closed())
                {
                    active.mailbox.replace(turn)?;
                    return Ok(receiver);
                }
            }

            let previous = session.take();
            let generation = Uuid::new_v4();
            let mailbox = Arc::new(AgentSessionMailbox::new());
            mailbox.replace(turn)?;
            let (shutdown, shutdown_receiver) = watch::channel(false);
            *session = Some(ActiveAgentSession {
                generation,
                identity: identity.clone(),
                mailbox: Arc::clone(&mailbox),
                shutdown,
            });
            (previous, (generation, identity, mailbox, shutdown_receiver))
        };
        if let Some(previous) = previous {
            let _ = previous.shutdown.send(true);
            previous.mailbox.close(
                "Agent session was replaced because its operation or configuration changed",
            )?;
        }
        let (generation, identity, mailbox, shutdown) = spawn;
        tauri::async_runtime::spawn(crate::agent::run_persistent_session_actor(
            app, generation, identity, mailbox, shutdown,
        ));
        Ok(receiver)
    }

    /// An App follow-up can only reuse the exact live actor and never cancels a turn.
    pub(crate) fn submit_app_message<R: tauri::Runtime>(
        &self,
        app: &AppHandle<R>,
        descriptor: &usecase::model::McpAppDescriptor,
        prompt: Vec<agent_client_protocol::schema::v1::ContentBlock>,
    ) -> Result<(), String> {
        let state = app.state::<AppState>();
        let material = state.lens_prompt_material(descriptor.operation_id)?;
        let projection =
            LensAgentProjection::from_input(&material.input, &material.target_set, &material.media)
                .map_err(|_| "Unable to construct App follow-up source")?;
        let snapshot = state.runtime.read().map_err(|_| "Lens state unavailable")?;
        if snapshot.lens.operation_id != Some(descriptor.operation_id)
            || snapshot.lens.stage != crate::model::LensStage::Completed
            || !(snapshot.lens.mcp_apps.iter().any(|a| a.id == descriptor.id)
                || snapshot
                    .lens
                    .response_history
                    .responses
                    .iter()
                    .any(|r| r.mcp_apps.iter().any(|a| a.id == descriptor.id)))
            || snapshot.config != material.config
            || snapshot.lens.projection.as_ref() != Some(&material.projection)
            || snapshot.lens.response_history.capacity_reached
            || snapshot
                .lens
                .live
                .as_ref()
                .is_some_and(|l| l.lifecycle != crate::model::LensMonitoringLifecycle::Watching)
        {
            return Err("App is stale, paused, or the Agent is busy".into());
        }
        let active = self
            .active
            .lock()
            .map_err(|_| "Agent control unavailable")?;
        if active.is_some() || self.unfinished_turns.load(Ordering::Acquire) != 0 {
            return Err("Agent is busy; retry after the current turn".into());
        }
        let session = self
            .session
            .lock()
            .map_err(|_| "Agent session unavailable")?;
        let session = session
            .as_ref()
            .filter(|s| {
                s.identity.operation_id == descriptor.operation_id
                    && s.identity
                        .config
                        .same_active_session_config(&snapshot.config)
                    && !s.mailbox.is_closed()
            })
            .ok_or("App live session is no longer available")?;
        let mut turn = AgentSessionTurn {
            context_revision: material.input.context_revision,
            projection_ref: material.projection,
            projection,
            app_prompt: Some(prompt),
            completion: None,
            work: None,
        };
        turn.track_work(Arc::clone(&self.unfinished_turns));
        session.mailbox.admit_app(turn)?;
        Ok(())
    }

    pub fn shutdown_session(
        &self,
        expected_operation_id: Option<Uuid>,
        reason: &str,
    ) -> Result<bool, String> {
        let active = {
            let mut session = self
                .session
                .lock()
                .map_err(|_| "Agent session control lock is poisoned".to_string())?;
            if session.as_ref().is_some_and(|active| {
                expected_operation_id
                    .is_some_and(|operation_id| active.identity.operation_id != operation_id)
            }) {
                return Ok(false);
            }
            session.take()
        };
        let Some(active) = active else {
            return Ok(false);
        };
        let _ = active.shutdown.send(true);
        active.mailbox.close(reason)?;
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn has_active_session(&self) -> bool {
        self.session.lock().expect("Agent session state").is_some()
    }

    #[cfg(test)]
    pub(crate) fn active_session_generation(&self) -> Option<Uuid> {
        self.session
            .lock()
            .expect("Agent session state")
            .as_ref()
            .map(|session| session.generation)
    }

    pub(crate) fn finish_session(&self, generation: Uuid) -> Result<bool, String> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| "Agent session control lock is poisoned".to_string())?;
        if session
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            *session = None;
            return Ok(true);
        }
        Ok(false)
    }
}

#[derive(Clone, Default)]
pub struct PickerControl {
    active: Arc<Mutex<Option<ActivePicker>>>,
}

struct ActivePicker {
    operation_id: Uuid,
    cancellation: watch::Sender<bool>,
}

pub struct PickerLease {
    active: Arc<Mutex<Option<ActivePicker>>>,
    cancellation: watch::Receiver<bool>,
}

impl PickerControl {
    pub fn try_begin(&self, operation_id: Uuid) -> Result<PickerLease, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "Picker control is unavailable")?;
        if active.is_some() {
            return Err("Lens Target picker is already active".into());
        }
        let (sender, cancellation) = watch::channel(false);
        *active = Some(ActivePicker {
            operation_id,
            cancellation: sender,
        });
        Ok(PickerLease {
            active: Arc::clone(&self.active),
            cancellation,
        })
    }

    pub fn cancel(&self, operation_id: Uuid) -> Result<(), String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "Picker control is unavailable")?;
        if let Some(picker) = active
            .as_ref()
            .filter(|picker| picker.operation_id == operation_id)
        {
            picker.cancellation.send_replace(true);
        }
        Ok(())
    }
}

impl PickerLease {
    /// Cancellation is checked before polling native presentation, including when Stop
    /// happened after state publication but before the first native future poll.
    pub async fn wait_for<T>(&mut self, future: impl std::future::Future<Output = T>) -> Option<T> {
        let cancelled = async {
            loop {
                if *self.cancellation.borrow_and_update() {
                    return;
                }
                if self.cancellation.changed().await.is_err() {
                    return;
                }
            }
        };
        tokio::select! {
            biased;
            _ = cancelled => None,
            result = future => Some(result),
        }
    }
}

impl Drop for PickerLease {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            *active = None;
        }
    }
}

#[derive(Default)]
struct ActiveLensMedia {
    operation_id: Option<Uuid>,
    context_revision: Option<u64>,
    payloads: BTreeMap<String, LensMediaPayload>,
}

#[derive(Default)]
pub struct LensMediaStore {
    active: Mutex<ActiveLensMedia>,
}

impl LensMediaStore {
    pub fn begin(&self, operation_id: Uuid) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        active.operation_id = Some(operation_id);
        active.context_revision = None;
        active.payloads.clear();
        Ok(())
    }

    pub fn replace(
        &self,
        operation_id: Uuid,
        payloads: Vec<LensMediaPayload>,
    ) -> Result<bool, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        if active.operation_id != Some(operation_id) {
            return Ok(false);
        }
        let next = validated_payload_map(payloads)?;
        active.context_revision = None;
        active.payloads = next;
        Ok(true)
    }

    pub fn replace_context(
        &self,
        operation_id: Uuid,
        context_revision: u64,
        payloads: Vec<LensMediaPayload>,
    ) -> Result<bool, String> {
        if context_revision == 0 {
            return Err("Lens media context revision must be non-zero".into());
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        if active.operation_id != Some(operation_id) {
            return Ok(false);
        }
        let next = validated_payload_map(payloads)?;
        active.context_revision = Some(context_revision);
        active.payloads = next;
        Ok(true)
    }

    pub fn payloads(&self, operation_id: Uuid) -> Result<Vec<LensMediaPayload>, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        if active.operation_id != Some(operation_id) {
            return Err("Lens media operation was superseded".into());
        }
        Ok(active.payloads.values().cloned().collect())
    }

    #[cfg(test)]
    pub fn payloads_for_context(
        &self,
        operation_id: Uuid,
        context_revision: u64,
    ) -> Result<Vec<LensMediaPayload>, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        if active.operation_id != Some(operation_id)
            || active.context_revision != Some(context_revision)
        {
            return Err("Lens media context was superseded".into());
        }
        Ok(active.payloads.values().cloned().collect())
    }

    pub fn payload_for_uri(&self, uri: &str) -> Result<Option<LensMediaPayload>, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        Ok(active
            .payloads
            .values()
            .find(|payload| payload.uri == uri)
            .cloned())
    }
}

pub struct AppState {
    pub(crate) publication: crate::publication::PublicationStore,
    pub(crate) mcp_apps: Arc<crate::mcp_apps::McpAppsStore>,
    pub(crate) history_store: std::sync::OnceLock<Arc<crate::session_history_store::HistoryStore>>,
    pub(crate) history_storage_error: std::sync::OnceLock<String>,
    pub(crate) history_writer: std::sync::OnceLock<crate::history_writer::HistoryWriter>,
    pub(crate) session_view: crate::session_view::SessionViewStore,
    pub platform: crate::platform::Services,
    pub session_controls: Mutex<Option<Arc<crate::session_controls::SessionControls>>>,
    pub(crate) runtime: RwLock<AppSnapshot>,
    pub agent_runtime_install: AsyncMutex<()>,
    pub agent_control: AgentControl,
    pub live_control: crate::live_runtime::LensLiveControl,
    pub lens_media: LensMediaStore,
    pub picker_control: PickerControl,
    pub store: ConfigStore,
}

#[derive(Debug, Clone)]
pub struct LensPromptMaterial {
    pub operation_id: Uuid,
    pub target_set: LensTargetSet,
    pub input: Arc<LensInput>,
    pub projection: ProjectionRef,
    pub media: Vec<LensMediaPayload>,
    pub config: AppConfig,
}

impl AppState {
    pub(crate) fn with_config(
        platform: crate::platform::Services,
        store: ConfigStore,
        config: AppConfig,
    ) -> Self {
        Self {
            publication: crate::publication::PublicationStore::default(),
            mcp_apps: Arc::new(crate::mcp_apps::McpAppsStore::default()),
            session_view: crate::session_view::SessionViewStore::default(),
            history_store: std::sync::OnceLock::new(),
            history_storage_error: std::sync::OnceLock::new(),
            history_writer: std::sync::OnceLock::new(),
            platform,
            runtime: RwLock::new(AppSnapshot::new(config)),
            session_controls: Mutex::new(None),
            agent_runtime_install: AsyncMutex::new(()),
            agent_control: AgentControl::default(),
            live_control: crate::live_runtime::LensLiveControl::default(),
            lens_media: LensMediaStore::default(),
            picker_control: PickerControl::default(),
            store,
        }
    }

    pub fn snapshot(&self) -> Result<AppSnapshot, String> {
        self.runtime
            .read()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| "application state lock is poisoned".to_string())
    }

    pub fn config(&self) -> Result<AppConfig, String> {
        self.runtime
            .read()
            .map(|snapshot| snapshot.config.clone())
            .map_err(|_| "application state lock is poisoned".to_string())
    }

    pub fn agent_selection(&self) -> Result<AgentSelectionState, String> {
        self.runtime
            .read()
            .map(|snapshot| snapshot.agent_selection.clone())
            .map_err(|_| "application state lock is poisoned".to_string())
    }

    pub fn lens(&self) -> Result<LensState, String> {
        self.runtime
            .read()
            .map(|snapshot| snapshot.lens.clone())
            .map_err(|_| "application state lock is poisoned".to_string())
    }

    /// Reads the complete Agent-bound input and its exact media revision under one lock order.
    pub fn lens_prompt_material(
        &self,
        expected_operation_id: Uuid,
    ) -> Result<LensPromptMaterial, String> {
        let snapshot = self
            .runtime
            .read()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        let lens = &snapshot.lens;
        if lens.operation_id != Some(expected_operation_id) {
            return Err("Lens operation was superseded before transformation started".into());
        }
        let target_set = lens
            .target_set
            .clone()
            .ok_or_else(|| "the current Lens operation has no fixed target set".to_string())?;
        let input = lens
            .input
            .clone()
            .ok_or_else(|| "the current Lens operation has no usable LensInput".to_string())?;
        let projection = lens
            .projection
            .clone()
            .ok_or_else(|| "the current Lens operation has no Agent projection".to_string())?;
        let active = self
            .lens_media
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        if active.operation_id != Some(expected_operation_id)
            || active.context_revision != Some(input.context_revision)
        {
            return Err("Lens media context was superseded".into());
        }
        Ok(LensPromptMaterial {
            operation_id: expected_operation_id,
            target_set,
            input,
            projection,
            media: active.payloads.values().cloned().collect(),
            config: snapshot.config.clone(),
        })
    }
}

pub fn publish_agent_runtime<R: tauri::Runtime>(
    app: &AppHandle<R>,
    next: AgentRuntimeState,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        advance_revision(&mut snapshot)?;
        snapshot.agent_runtime = next;
        snapshot.clone()
    };
    emit_app_snapshot(app, snapshot, false)
}

pub fn update_agent_runtime<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    update: impl FnOnce(&mut AgentRuntimeState),
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        if snapshot.agent_runtime.operation_id != Some(operation_id) {
            return Ok(false);
        }
        advance_revision(&mut snapshot)?;
        update(&mut snapshot.agent_runtime);
        snapshot.clone()
    };
    emit_app_snapshot(app, snapshot, false)?;
    Ok(true)
}

pub(crate) fn emit_app_snapshot<R: tauri::Runtime>(
    app: &AppHandle<R>,
    snapshot: AppSnapshot,
    sync_tray: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.mcp_apps.sync_operation(&state)?;
    if sync_tray {
        crate::ui::sync_tray_menu(app)?;
    }
    crate::publication::publish(app, snapshot)
}

pub fn publish_agent_selection<R: tauri::Runtime>(
    app: &AppHandle<R>,
    next: AgentSelectionState,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        advance_revision(&mut snapshot)?;
        snapshot.agent_selection = next;
        snapshot.clone()
    };
    emit_app_snapshot(app, snapshot, true)
}

pub fn update_agent_selection<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    update: impl FnOnce(&mut AgentSelectionState),
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        if snapshot.agent_selection.operation_id != Some(operation_id) {
            return Ok(false);
        }
        advance_revision(&mut snapshot)?;
        update(&mut snapshot.agent_selection);
        snapshot.clone()
    };
    emit_app_snapshot(app, snapshot, true)?;
    Ok(true)
}

pub fn publish_lens_state<R: tauri::Runtime>(
    app: &AppHandle<R>,
    next: LensState,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (snapshot, sync_tray) = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        let sync_tray =
            (snapshot.lens.stage == LensStage::Selecting) != (next.stage == LensStage::Selecting);
        advance_revision(&mut snapshot)?;
        snapshot.lens = next;
        (snapshot.clone(), sync_tray)
    };
    emit_app_snapshot(app, snapshot, sync_tray)
}

/// Atomically commits the first canonical context and its exact private media payload revision.
pub fn commit_initial_lens_context<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    next: LensState,
    payloads: Vec<LensMediaPayload>,
) -> Result<bool, String> {
    validate_context_state(operation_id, &next)?;
    let state = app.state::<AppState>();
    let (snapshot, sync_tray) = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        if snapshot.lens.operation_id != Some(operation_id) || snapshot.lens.context.is_some() {
            return Ok(false);
        }
        let context_revision = next
            .context
            .as_ref()
            .expect("validated context state has context")
            .revision;
        let next_payloads = validated_payload_map(payloads)?;
        let mut active = state
            .lens_media
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        if active.operation_id != Some(operation_id) {
            return Ok(false);
        }
        let sync_tray = snapshot.lens.live.is_some() != next.live.is_some();
        let revision = next_revision(&snapshot)?;
        active.context_revision = Some(context_revision);
        active.payloads = next_payloads;
        snapshot.revision = revision;
        snapshot.lens = next;
        (snapshot.clone(), sync_tray)
    };
    emit_app_snapshot(app, snapshot, sync_tray)?;
    Ok(true)
}

/// Atomically merges a refreshed canonical context into the latest Agent and representation state.
pub(crate) fn commit_lens_context_refresh<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    context_id: Uuid,
    expected_previous_context_revision: u64,
    refresh: LensContextRefreshCommit,
    payloads: Vec<LensMediaPayload>,
) -> Result<bool, String> {
    validate_refresh_identity(
        operation_id,
        context_id,
        expected_previous_context_revision,
        &refresh,
    )?;
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        let Some(next) = prepare_context_refresh(
            &snapshot.lens,
            operation_id,
            context_id,
            expected_previous_context_revision,
            refresh,
        )?
        else {
            return Ok(false);
        };
        let context_revision = next
            .context
            .as_ref()
            .expect("validated refresh has context")
            .revision;
        let next_payloads = validated_payload_map(payloads)?;
        let mut active = state
            .lens_media
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        if active.operation_id != Some(operation_id) {
            return Ok(false);
        }
        let revision = next_revision(&snapshot)?;
        active.context_revision = Some(context_revision);
        active.payloads = next_payloads;
        snapshot.revision = revision;
        snapshot.lens = next;
        snapshot.clone()
    };
    emit_app_snapshot(app, snapshot, false)?;
    Ok(true)
}

/// Clears the exact active operation and its private media in one publication boundary.
pub fn clear_lens_operation<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        if snapshot.lens.operation_id != Some(operation_id) {
            return Ok(false);
        }
        let mut active = state
            .lens_media
            .active
            .lock()
            .map_err(|_| "Lens media store lock is poisoned".to_string())?;
        if active.operation_id != Some(operation_id) {
            return Ok(false);
        }
        let revision = next_revision(&snapshot)?;
        active.operation_id = None;
        active.context_revision = None;
        active.payloads.clear();
        snapshot.revision = revision;
        snapshot.lens = LensState::default();
        snapshot.clone()
    };
    emit_app_snapshot(app, snapshot, true)?;
    Ok(true)
}

pub fn begin_agent_run<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    expected_projection: &ProjectionRef,
    expected_config: &AppConfig,
    initialize: impl FnOnce(Uuid, &mut LensState),
) -> Result<AgentRunHandle, String> {
    let state = app.state::<AppState>();
    let (run, snapshot) = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        if snapshot.lens.response_history.capacity_reached {
            return Err(usecase::response_history::RESPONSE_HISTORY_CAPACITY_MESSAGE.into());
        }
        if snapshot.lens.operation_id != Some(operation_id)
            || snapshot.lens.projection.as_ref() != Some(expected_projection)
            || !snapshot.config.same_active_session_config(expected_config)
            || snapshot
                .lens
                .live
                .as_ref()
                .is_some_and(|live| live.lifecycle != LensMonitoringLifecycle::Watching)
        {
            return Err(
                "Lens operation, projection, configuration, or monitoring authority was superseded before the Agent run started"
                    .into(),
            );
        }
        let revision = next_revision(&snapshot)?;
        let run = state.agent_control.begin(operation_id)?;
        snapshot.lens.prompt_execution_revision = expected_config.prompt_presets.execution_revision;
        initialize(run.key.run_id, &mut snapshot.lens);
        let agent = snapshot
            .lens
            .agent
            .as_mut()
            .ok_or_else(|| "Agent run initialization did not publish run state".to_string())?;
        agent.input_projection = Some(expected_projection.clone());
        snapshot.revision = revision;
        (run, snapshot.clone())
    };
    if let Err(error) = emit_app_snapshot(app, snapshot, false) {
        state.agent_control.finish(run.key)?;
        return Err(error);
    }
    Ok(run)
}

pub fn update_lens_state<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    update_lens_state_if(
        app,
        |snapshot| snapshot.lens.operation_id == Some(operation_id),
        update,
    )
}

pub fn update_lens_state_for_context<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    context_id: Uuid,
    context_revision: u64,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    update_lens_state_if(
        app,
        |snapshot| {
            snapshot.lens.operation_id == Some(operation_id)
                && snapshot.lens.context.as_ref().is_some_and(|context| {
                    context.context_id == context_id && context.revision == context_revision
                })
        },
        update,
    )
}

pub fn update_lens_state_for_projection<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    expected_projection: &ProjectionRef,
    expected_config: &AppConfig,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    update_lens_state_if(
        app,
        |snapshot| {
            snapshot.config.same_active_session_config(expected_config)
                && snapshot.lens.operation_id == Some(operation_id)
                && snapshot.lens.projection.as_ref() == Some(expected_projection)
        },
        update,
    )
}

pub fn update_lens_state_for_run<R: tauri::Runtime>(
    app: &AppHandle<R>,
    key: AgentRunKey,
    expected_config: &AppConfig,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    update_lens_state_if(
        app,
        |snapshot| agent_run_has_authority(snapshot, key, expected_config),
        update,
    )
}

fn update_lens_state_if<R: tauri::Runtime>(
    app: &AppHandle<R>,
    predicate: impl FnOnce(&AppSnapshot) -> bool,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let (snapshot, sync_tray) = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        if !predicate(&snapshot) {
            return Ok(false);
        }
        let was_selecting = snapshot.lens.stage == LensStage::Selecting;
        advance_revision(&mut snapshot)?;
        update(&mut snapshot.lens);
        let sync_tray = was_selecting != (snapshot.lens.stage == LensStage::Selecting);
        (snapshot.clone(), sync_tray)
    };
    emit_app_snapshot(app, snapshot, sync_tray)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset_app(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .manage(state)
            .build(crate::product_context())
            .unwrap()
    }

    #[test]
    fn prompt_metadata_edits_preserve_active_work_but_instruction_changes_cancel_it() {
        use usecase::prompt_presets::PromptPresetMutation;
        let state = crate::test_support::state();
        let run = state.agent_control.begin(Uuid::new_v4()).unwrap();
        let original = state.config().unwrap();
        let app = preset_app(state);
        let preset = original.prompt_presets.selected();
        let (updated, restart) = crate::commands::commit_prompt_presets(
            app.handle(),
            PromptPresetMutation::Update {
                id: preset.id.clone(),
                expected_revision: preset.revision,
                name: "Renamed".into(),
                template: preset.template.clone(),
            },
        )
        .unwrap();
        assert!(restart.is_none());
        assert!(!*run.cancellation.borrow());
        assert!(original.same_execution_config(&updated.config));
        assert_eq!(
            app.state::<AppState>().store.load().prompt_presets,
            updated.config.prompt_presets
        );
        let (changed, _) = crate::commands::commit_prompt_presets(
            app.handle(),
            PromptPresetMutation::Select {
                id: "practical".into(),
            },
        )
        .unwrap();
        assert!(*run.cancellation.borrow());
        assert!(!original.same_execution_config(&changed.config));
        assert_eq!(
            changed.lens.prompt_execution_revision,
            changed.config.prompt_presets.execution_revision
        );
        let (same, restart) = crate::commands::commit_prompt_presets(
            app.handle(),
            PromptPresetMutation::Select {
                id: "practical".into(),
            },
        )
        .unwrap();
        assert_eq!(same.revision, changed.revision);
        assert!(restart.is_none());
    }

    #[test]
    fn failed_prompt_persistence_preserves_configuration_and_running_work() {
        use usecase::prompt_presets::PromptPresetMutation;
        let path =
            std::env::temp_dir().join(format!("lens-preset-save-failure-{}", Uuid::new_v4()));
        std::fs::write(&path, "not a directory").unwrap();
        let mut state = crate::test_support::state();
        state.store = ConfigStore::at_path(path.join("settings.json"));
        let original = state.snapshot().unwrap();
        let run = state.agent_control.begin(Uuid::new_v4()).unwrap();
        let app = preset_app(state);
        assert!(crate::commands::commit_prompt_presets(
            app.handle(),
            PromptPresetMutation::Select {
                id: "practical".into(),
            }
        )
        .is_err());
        let current = app.state::<AppState>().snapshot().unwrap();
        assert_eq!(current.config, original.config);
        assert_eq!(current.revision, original.revision);
        assert!(!*run.cancellation.borrow());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cancelled_agent_run_remains_pending_until_finished() {
        let control = AgentControl::default();
        let run = control.begin(Uuid::new_v4()).unwrap();
        assert!(control.has_pending_work().unwrap());
        assert_eq!(control.cancel_active().unwrap(), Some(run.key));
        assert!(*run.cancellation.borrow());
        assert!(control.has_pending_work().unwrap());
        assert!(control.finish(run.key).unwrap());
        assert!(!control.has_pending_work().unwrap());
    }

    #[test]
    fn dropped_run_lifetime_clears_only_its_own_run() {
        let control = AgentControl::default();
        let operation = Uuid::new_v4();
        let first = control.begin(operation).unwrap();
        let first_lifetime = control.run_lifetime(first.key);
        let second = control.begin(operation).unwrap();
        let second_lifetime = control.run_lifetime(second.key);
        drop(first_lifetime);
        assert!(control.has_pending_work().unwrap());
        assert_eq!(
            control.active.lock().unwrap().as_ref().unwrap().key,
            second.key
        );
        drop(second_lifetime);
        assert!(!control.has_pending_work().unwrap());
    }

    #[tokio::test]
    async fn app_message_rpc_acknowledges_queue_admission_with_latest_context_and_rejects_duplicates(
    ) {
        use adapter_mcp_server::apps::{AppArtifact, AppBroker, AppResource};
        use serde_json::json;
        let state = crate::test_support::state();
        let (operation, targets, context, input) = crate::mcp_apps_validation::fixture().unwrap();
        let projection = LensAgentProjection::from_input(&input, &targets, &[]).unwrap();
        let projection_ref = projection.projection_ref(std::num::NonZeroU64::new(1).unwrap());
        let context_id = input.context_id;
        let run_id = Uuid::new_v4();
        let config = state.config().unwrap();
        state.lens_media.begin(operation).unwrap();
        state
            .lens_media
            .replace_context(operation, input.context_revision, vec![])
            .unwrap();
        state.runtime.write().unwrap().lens = LensState {
            operation_id: Some(operation), stage: LensStage::Completed,
            target_set: Some(targets), context: Some(context.into()), input: Some(input.into()),
            projection: Some(projection_ref),
            agent: Some(serde_json::from_value(json!({"run_id":run_id,"kind":"codex","adapter_name":"fixture","adapter_version":"1","session_id":"direct-message-session","received_updates":0})).unwrap()),
            ..Default::default()
        };
        let mailbox = Arc::new(AgentSessionMailbox::new());
        let (shutdown, _shutdown_receiver) = watch::channel(false);
        *state.agent_control.session.lock().unwrap() = Some(ActiveAgentSession {
            generation: Uuid::new_v4(),
            identity: AgentSessionIdentity {
                operation_id: operation,
                context_id,
                effective_working_directory: config.working_directory.clone(),
                config: config.clone(),
            },
            mailbox: mailbox.clone(),
            shutdown,
        });
        let broker = Arc::new(
            AppBroker::start(vec![], "<p>shell</p>".into())
                .await
                .unwrap(),
        );
        let source = state
            .mcp_apps
            .install(operation, "direct-message-session".into(), &broker, &config)
            .unwrap();
        let app = preset_app(state);
        let handle = app.handle().clone();
        let state = app.state::<AppState>();
        let descriptors = state
            .mcp_apps
            .retain(
                &handle,
                operation,
                "direct-message-session",
                vec![AppArtifact {
                    id: Uuid::new_v4(),
                    run_id,
                    server_id: "fixture-source".into(),
                    tool_name: "fixture-tool".into(),
                    resource_uri: "ui://fixture".into(),
                    title: "App".into(),
                    resource: AppResource {
                        html: "<p>App</p>".into(),
                        meta: json!({}),
                    },
                    input: json!({}),
                    result: json!({"content":[]}),
                }],
            )
            .unwrap();
        state.runtime.write().unwrap().lens.mcp_apps = descriptors.clone();
        let overlay = tauri::WebviewWindowBuilder::new(
            &app,
            crate::ui::LENS_WINDOW_LABEL,
            Default::default(),
        )
        .build()
        .unwrap();
        let origin = overlay.url().unwrap().origin().ascii_serialization();
        let opened =
            crate::mcp_apps::open_mcp_app(handle.clone(), overlay, descriptors[0].id, origin)
                .await
                .unwrap();
        assert!(opened.live);
        for value in [10, 73] {
            crate::mcp_apps::mcp_app_request(handle.clone(), opened.id, json!({"method":"ui/update-model-context","params":{"structuredContent":{"selected_value":value}}})).await.unwrap();
        }
        let message = json!({"method":"ui/message","params":{"role":"user","content":[{"type":"text","text":"Explain selected value"}]}});
        let ack = crate::mcp_apps::mcp_app_request(handle.clone(), opened.id, message.clone())
            .await
            .unwrap();
        assert_eq!(serde_json::to_value(ack).unwrap(), json!({"result":{}}));
        // The turn has not executed: protocol success is queue admission, not its answer.
        assert!(mailbox.has_app_message());
        assert!(
            crate::mcp_apps::mcp_app_request(handle.clone(), opened.id, message.clone())
                .await
                .is_err()
        );
        let first = mailbox.take_pending().unwrap().unwrap();
        assert!(first.completion.is_none());
        let prompt = serde_json::to_value(&first.app_prompt).unwrap();
        assert!(prompt[0]["text"]
            .as_str()
            .unwrap()
            .contains("Explain selected value"));
        let context = prompt[1]["text"].as_str().unwrap();
        assert!(context.contains("\"selected_value\":73"));
        assert!(!context.contains("\"selected_value\":10"));
        // Dequeue does not clear unfinished work before the actor starts the turn.
        assert!(
            crate::mcp_apps::mcp_app_request(handle.clone(), opened.id, message.clone())
                .await
                .is_err()
        );
        first.complete(Ok(AgentSessionTurnCompletion::Finished));
        crate::mcp_apps::mcp_app_request(
            handle.clone(),
            opened.id,
            json!({"method":"ui/update-model-context","params":{}}),
        )
        .await
        .unwrap();
        crate::mcp_apps::mcp_app_request(handle.clone(), opened.id, message.clone())
            .await
            .unwrap();
        let second = mailbox.take_pending().unwrap().unwrap();
        let cleared = serde_json::to_value(&second.app_prompt)
            .unwrap()
            .to_string();
        assert!(!cleared.contains("selected_value"));
        second.complete(Err("Agent execution failed later".into()));
        assert!(!state.agent_control.has_pending_work().unwrap());
        state
            .runtime
            .write()
            .unwrap()
            .lens
            .response_history
            .capacity_reached = true;
        assert!(
            crate::mcp_apps::mcp_app_request(handle.clone(), opened.id, message.clone())
                .await
                .is_err()
        );
        assert!(!mailbox.has_app_message());
        state
            .runtime
            .write()
            .unwrap()
            .lens
            .response_history
            .capacity_reached = false;
        state
            .agent_control
            .shutdown_session(Some(operation), "session retired")
            .unwrap();
        assert!(
            crate::mcp_apps::mcp_app_request(handle.clone(), opened.id, message)
                .await
                .is_err()
        );
        crate::mcp_apps::close_mcp_app(handle, opened.id).unwrap();
        drop(source);
        broker.close().await;
    }

    #[test]
    fn retained_idle_agent_session_does_not_require_quit_confirmation() {
        let control = AgentControl::default();
        let generation = Uuid::new_v4();
        let (shutdown, _receiver) = watch::channel(false);
        *control.session.lock().unwrap() = Some(ActiveAgentSession {
            generation,
            identity: AgentSessionIdentity {
                operation_id: Uuid::new_v4(),
                context_id: Uuid::new_v4(),
                effective_working_directory: std::path::PathBuf::from("/tmp"),
                config: AppConfig::new(std::path::PathBuf::from("/tmp")),
            },
            mailbox: Arc::new(AgentSessionMailbox::new()),
            shutdown,
        });
        assert!(control.session.lock().unwrap().is_some());
        assert!(!control.has_pending_work().unwrap());
        assert!(control.finish_session(generation).unwrap());
    }

    #[test]
    fn newer_agent_run_with_same_operation_cancels_and_outlives_previous_run() {
        let control = AgentControl::default();
        let operation_id = Uuid::new_v4();
        let mut first = control.begin(operation_id).expect("first run");
        let mut second = control.begin(operation_id).expect("second run");

        assert_ne!(first.key.run_id, second.key.run_id);
        assert!(*first.cancellation.borrow_and_update());
        assert!(!control.finish(first.key).expect("finish stale run"));
        assert!(!control.cancel(first.key).expect("cancel stale run"));
        assert!(control.cancel(second.key).expect("cancel active run"));
        assert!(second
            .cancellation
            .has_changed()
            .expect("second cancellation change"));
        assert!(*second.cancellation.borrow_and_update());
    }

    #[test]
    fn picker_lease_rejects_reentry_and_releases_on_drop() {
        let control = PickerControl::default();
        let operation = Uuid::new_v4();
        let lease = control.try_begin(operation).expect("first picker lease");
        assert!(control.try_begin(operation).is_err());
        drop(lease);
        assert!(control.try_begin(operation).is_ok());
    }

    #[tokio::test]
    async fn picker_cancel_before_presentation_never_polls_native_work() {
        let control = PickerControl::default();
        let operation = Uuid::new_v4();
        let mut lease = control.try_begin(operation).unwrap();
        control.cancel(operation).unwrap();
        assert_eq!(
            lease
                .wait_for(async { panic!("cancelled picker was presented") })
                .await,
            None::<()>
        );
        // Cancellation cannot release exclusion before the old native future is dropped.
        assert!(control.try_begin(operation).is_err());
        drop(lease);
        let mut next = control.try_begin(operation).unwrap();
        assert_eq!(next.wait_for(async { 42 }).await, Some(42));
    }

    #[tokio::test]
    async fn picker_cancel_drops_suspended_native_work_before_reentry() {
        struct PendingNative(Arc<AtomicBool>);
        impl Drop for PendingNative {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let control = PickerControl::default();
        let operation = Uuid::new_v4();
        let mut lease = control.try_begin(operation).unwrap();
        let dropped = Arc::new(AtomicBool::new(false));
        let native = PendingNative(dropped.clone());
        let (started, entered) = oneshot::channel();
        let work = async move {
            let _native = native;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        };
        let wait = lease.wait_for(work);
        let cancellation = async {
            entered.await.unwrap();
            control.cancel(Uuid::new_v4()).unwrap();
            assert!(!dropped.load(Ordering::Acquire));
            assert!(control.try_begin(operation).is_err());
            control.cancel(operation).unwrap();
        };
        let (result, ()) = tokio::join!(wait, cancellation);
        assert_eq!(result, None);
        assert!(dropped.load(Ordering::Acquire));
        drop(lease);
        assert!(control.try_begin(operation).is_ok());
    }

    #[test]
    fn media_payloads_are_finite_and_scoped_to_the_current_operation() {
        let store = LensMediaStore::default();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let payload = LensMediaPayload {
            attachment_id: "media-node-1".into(),
            uri: "lens://fixture/media-node-1".into(),
            mime_type: "image/png".into(),
            data: "iVBORw0KGgo=".into(),
        };

        store.begin(first).expect("begin first operation");
        assert!(store
            .replace(first, vec![payload.clone()])
            .expect("store first payload"));
        assert_eq!(
            store.payloads(first).expect("first payloads"),
            vec![payload.clone()]
        );
        assert_eq!(
            store
                .payload_for_uri(&payload.uri)
                .expect("lookup active payload"),
            Some(payload.clone())
        );

        store.begin(second).expect("begin second operation");
        assert!(store.payloads(first).is_err());
        assert!(store.payloads(second).expect("second payloads").is_empty());
        assert!(store
            .payload_for_uri(&payload.uri)
            .expect("old URI is no longer active")
            .is_none());
        assert!(!store
            .replace(first, Vec::new())
            .expect("reject superseded payloads"));
    }

    #[test]
    fn media_store_rejects_duplicate_attachment_uris() {
        let store = LensMediaStore::default();
        let operation_id = Uuid::new_v4();
        store.begin(operation_id).expect("begin operation");
        let payload = LensMediaPayload {
            attachment_id: "media-node-1".into(),
            uri: "lens://fixture/media-node-1".into(),
            mime_type: "image/png".into(),
            data: "iVBORw0KGgo=".into(),
        };
        let duplicate = LensMediaPayload {
            attachment_id: "media-node-2".into(),
            ..payload.clone()
        };

        assert!(store
            .replace(operation_id, vec![payload, duplicate])
            .expect_err("duplicate URI must fail")
            .contains("URI is duplicated"));
    }

    #[test]
    fn media_payload_lookup_requires_the_exact_context_revision() {
        let store = LensMediaStore::default();
        let operation_id = Uuid::new_v4();
        let payload = LensMediaPayload {
            attachment_id: "media-node-1".into(),
            uri: format!("lens://context/{operation_id}/4/media/media-node-1"),
            mime_type: "image/png".into(),
            data: "iVBORw0KGgo=".into(),
        };

        store.begin(operation_id).expect("begin operation");
        assert!(store
            .replace_context(operation_id, 4, vec![payload.clone()])
            .expect("commit context media"));
        assert_eq!(
            store
                .payloads_for_context(operation_id, 4)
                .expect("exact context payloads"),
            vec![payload]
        );
        assert!(store.payloads_for_context(operation_id, 3).is_err());
        assert!(store.payloads_for_context(operation_id, 5).is_err());
    }
}
