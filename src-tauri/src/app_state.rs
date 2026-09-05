use crate::{
    lens::{LensContext, LensInput, LensMediaPayload, LensTargetSet},
    live_sync::{LensAgentProjection, ProjectionRef},
    model::{
        AgentRuntimeState, AgentSelectionState, AppConfig, AppSnapshot, LensFreshness,
        LensMonitoringLifecycle, LensRefreshOutcome, LensSourceHealth, LensStage, LensState,
    },
    store::ConfigStore,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{oneshot, watch, Mutex as AsyncMutex, Notify};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRunKey {
    pub operation_id: Uuid,
    pub run_id: Uuid,
}

pub struct AgentRunHandle {
    pub key: AgentRunKey,
    pub cancellation: watch::Receiver<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LensContextRefreshOutcome {
    Unchanged,
    Updated,
}

pub(crate) struct LensContextRefreshCommit {
    pub target_set: LensTargetSet,
    pub context: LensContext,
    pub input: LensInput,
    pub projection: ProjectionRef,
    pub source_health: LensSourceHealth,
    pub outcome: LensContextRefreshOutcome,
}

struct ActiveAgentRun {
    key: AgentRunKey,
    cancellation: watch::Sender<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AgentSessionIdentity {
    pub operation_id: Uuid,
    pub context_id: Uuid,
    pub config: AppConfig,
}

pub(crate) struct AgentSessionTurn {
    pub context_revision: u64,
    pub projection_ref: ProjectionRef,
    pub projection: LensAgentProjection,
    completion: Option<oneshot::Sender<Result<AgentSessionTurnCompletion, String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentSessionTurnCompletion {
    Finished,
    Coalesced,
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
                completion: Some(completion),
            },
            receiver,
        )
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
            if self.closed.load(Ordering::Acquire) {
                return Err("Agent session mailbox is closed".into());
            }
            pending.replace(turn)
        };
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

    pub fn close(&self, reason: &str) -> Result<(), String> {
        self.closed.store(true, Ordering::Release);
        if let Some(pending) = self.take_pending()? {
            pending.complete(Err(reason.into()));
        }
        self.notify.notify_waiters();
        Ok(())
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
    active: Mutex<Option<ActiveAgentRun>>,
    session: Mutex<Option<ActiveAgentSession>>,
}

impl AgentControl {
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

    pub(crate) fn submit_session(
        &self,
        app: AppHandle,
        identity: AgentSessionIdentity,
        context_revision: u64,
        projection_ref: ProjectionRef,
        projection: LensAgentProjection,
    ) -> Result<oneshot::Receiver<Result<AgentSessionTurnCompletion, String>>, String> {
        let (turn, receiver) = AgentSessionTurn::new(context_revision, projection_ref, projection);
        let (previous, spawn) = {
            let mut session = self
                .session
                .lock()
                .map_err(|_| "Agent session control lock is poisoned".to_string())?;
            if let Some(active) = session.as_ref() {
                if active.identity == identity && !active.mailbox.is_closed() {
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
    active: Arc<AtomicBool>,
}

pub struct PickerLease {
    active: Arc<AtomicBool>,
}

impl PickerControl {
    pub fn try_begin(&self) -> Result<PickerLease, String> {
        self.active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Lens Target picker is already active".to_string())?;
        Ok(PickerLease {
            active: Arc::clone(&self.active),
        })
    }
}

impl Drop for PickerLease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
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
        let mut next = BTreeMap::new();
        let mut uris = BTreeSet::new();
        for payload in payloads {
            if !uris.insert(payload.uri.clone()) {
                return Err("Lens media attachment URI is duplicated".into());
            }
            if next
                .insert(payload.attachment_id.clone(), payload)
                .is_some()
            {
                return Err("Lens media attachment identity is duplicated".into());
            }
        }
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
        let mut next = BTreeMap::new();
        let mut uris = BTreeSet::new();
        for payload in payloads {
            if !uris.insert(payload.uri.clone()) {
                return Err("Lens media attachment URI is duplicated".into());
            }
            if next
                .insert(payload.attachment_id.clone(), payload)
                .is_some()
            {
                return Err("Lens media attachment identity is duplicated".into());
            }
        }
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
    pub input: LensInput,
    pub projection: ProjectionRef,
    pub media: Vec<LensMediaPayload>,
    pub config: AppConfig,
}

impl AppState {
    pub fn load(platform: crate::platform::Services) -> Self {
        let store = ConfigStore::new();
        let config = store.load();
        Self {
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
        self.snapshot().map(|snapshot| snapshot.config)
    }

    pub fn agent_selection(&self) -> Result<AgentSelectionState, String> {
        self.snapshot().map(|snapshot| snapshot.agent_selection)
    }

    pub fn lens(&self) -> Result<LensState, String> {
        self.snapshot().map(|snapshot| snapshot.lens)
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

pub fn publish_agent_runtime(app: &AppHandle, next: AgentRuntimeState) -> Result<(), String> {
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

pub fn update_agent_runtime(
    app: &AppHandle,
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

pub(crate) fn advance_revision(snapshot: &mut AppSnapshot) -> Result<(), String> {
    snapshot.revision = next_revision(snapshot)?;
    Ok(())
}

pub(crate) fn next_revision(snapshot: &AppSnapshot) -> Result<u32, String> {
    snapshot
        .revision
        .checked_add(1)
        .ok_or_else(|| "application state revision is exhausted".to_string())
}

pub(crate) fn emit_app_snapshot(
    app: &AppHandle,
    snapshot: AppSnapshot,
    sync_tray: bool,
) -> Result<(), String> {
    if sync_tray {
        crate::ui::sync_tray_menu(app)?;
    }
    app.emit("app-state-changed", snapshot)
        .map_err(|error| error.to_string())
}

pub fn publish_agent_selection(app: &AppHandle, next: AgentSelectionState) -> Result<(), String> {
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

pub fn update_agent_selection(
    app: &AppHandle,
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

pub fn publish_lens_state(app: &AppHandle, next: LensState) -> Result<(), String> {
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
pub fn commit_initial_lens_context(
    app: &AppHandle,
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

fn lens_context_refresh_has_authority(
    lens: &LensState,
    operation_id: Uuid,
    context_id: Uuid,
    expected_previous_context_revision: u64,
) -> bool {
    lens.operation_id == Some(operation_id)
        && lens.context.as_ref().is_some_and(|context| {
            context.context_id == context_id
                && context.revision == expected_previous_context_revision
        })
        && lens
            .live
            .as_ref()
            .is_some_and(|live| live.lifecycle == LensMonitoringLifecycle::Watching)
}

/// Atomically merges a refreshed canonical context into the latest Agent and representation state.
pub(crate) fn commit_lens_context_refresh(
    app: &AppHandle,
    operation_id: Uuid,
    context_id: Uuid,
    expected_previous_context_revision: u64,
    refresh: LensContextRefreshCommit,
    payloads: Vec<LensMediaPayload>,
) -> Result<bool, String> {
    let expected_context_revision = expected_previous_context_revision
        .checked_add(1)
        .ok_or_else(|| "Lens context revision is exhausted".to_string())?;
    if refresh.context.context_id != context_id
        || refresh.context.revision != expected_context_revision
        || refresh.target_set.selection_id != operation_id
    {
        return Err("refreshed Lens target/context identity or revision is inconsistent".into());
    }
    let state = app.state::<AppState>();
    let snapshot =
        {
            let mut snapshot = state
                .runtime
                .write()
                .map_err(|_| "application state lock is poisoned".to_string())?;
            if !lens_context_refresh_has_authority(
                &snapshot.lens,
                operation_id,
                context_id,
                expected_previous_context_revision,
            ) {
                return Ok(false);
            }
            let previous_projection =
                snapshot.lens.projection.as_ref().ok_or_else(|| {
                    "the current Lens operation has no Agent projection".to_string()
                })?;
            let previous_target_set =
                snapshot.lens.target_set.as_ref().ok_or_else(|| {
                    "the current Lens operation has no fixed target set".to_string()
                })?;
            if !previous_target_set.has_same_identity(&refresh.target_set) {
                return Err("refreshed Lens target identity changed during facts refresh".into());
            }
            if !refresh_projection_transition_is_valid(
                previous_projection,
                &refresh.projection,
                refresh.outcome,
            ) {
                return Err("refreshed Lens projection transition is inconsistent".into());
            }

            let mut next = snapshot.lens.clone();
            let context_revision = refresh.context.revision;
            next.target_set = Some(refresh.target_set);
            next.context = Some(refresh.context);
            next.input = Some(refresh.input);
            next.projection = Some(refresh.projection);
            reconcile_lens_after_context_refresh(
                &mut next,
                context_revision,
                refresh.source_health,
                refresh.outcome,
            );
            validate_context_state(operation_id, &next)?;

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
pub fn clear_lens_operation(app: &AppHandle, operation_id: Uuid) -> Result<bool, String> {
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

fn validate_context_state(operation_id: Uuid, lens: &LensState) -> Result<(), String> {
    if lens.operation_id != Some(operation_id) {
        return Err("canonical Lens state has a mismatched operation identity".into());
    }
    let context = lens
        .context
        .as_ref()
        .ok_or_else(|| "canonical Lens state must contain a context".to_string())?;
    let input = lens
        .input
        .as_ref()
        .ok_or_else(|| "canonical Lens state must contain a LensInput".to_string())?;
    let target_set = lens
        .target_set
        .as_ref()
        .ok_or_else(|| "canonical Lens state must contain a target set".to_string())?;
    if context.revision == 0
        || target_set.selection_id != operation_id
        || input.context_id != context.context_id
        || input.context_revision != context.revision
        || lens.projection.is_none()
    {
        return Err(
            "canonical Lens context, input, and projection revisions are inconsistent".into(),
        );
    }
    if target_set.targets.len() != context.sources.len()
        || target_set
            .targets
            .iter()
            .zip(&context.sources)
            .any(|(target, source)| {
                target.facts_revision == 0
                    || target.facts_revision > context.revision
                    || source.target_id != target.id
                    || source.source.application != target.facts.application_name
                    || source.source.window_title != target.facts.title
                    || source.source.bundle_id != target.identity.bundle_id
                    || source.source.window_id != target.identity.window_id
            })
    {
        return Err(
            "canonical Lens target facts and context source provenance are inconsistent".into(),
        );
    }
    Ok(())
}

fn reconcile_lens_after_context_refresh(
    lens: &mut LensState,
    context_revision: u64,
    source_health: LensSourceHealth,
    outcome: LensContextRefreshOutcome,
) {
    match outcome {
        LensContextRefreshOutcome::Updated => {
            let agent_work_is_active =
                matches!(lens.stage, LensStage::Connecting | LensStage::Transforming);
            if !agent_work_is_active {
                lens.pending_representation = None;
                lens.stage = LensStage::Ready;
            }
            lens.error = None;
            let freshness = if source_health == LensSourceHealth::Unavailable {
                LensFreshness::Unverified
            } else if lens.representation.is_some() {
                LensFreshness::Stale
            } else {
                LensFreshness::Checking
            };
            let live = lens
                .live
                .as_mut()
                .expect("a refreshed operation has explicit live state");
            live.health = source_health;
            live.freshness = freshness;
            live.last_outcome = None;
            live.error = None;
        }
        LensContextRefreshOutcome::Unchanged => {
            let representation_is_current = lens
                .representation
                .as_ref()
                .zip(lens.projection.as_ref())
                .is_some_and(|(representation, projection)| {
                    &representation.projection == projection
                });
            if representation_is_current {
                lens.representation
                    .as_mut()
                    .expect("a current representation exists")
                    .context_revision = context_revision;
            }

            let agent_work_is_active =
                matches!(lens.stage, LensStage::Connecting | LensStage::Transforming);
            let preserve_recovery = matches!(
                lens.stage,
                LensStage::AuthenticationRequired | LensStage::Cancelled | LensStage::Failed
            );
            if !preserve_recovery {
                lens.error = None;
            }
            let freshness = if source_health == LensSourceHealth::Unavailable {
                LensFreshness::Unverified
            } else if agent_work_is_active {
                LensFreshness::Checking
            } else if representation_is_current {
                LensFreshness::Current
            } else if lens.representation.is_some() {
                LensFreshness::Stale
            } else {
                LensFreshness::None
            };
            let live = lens
                .live
                .as_mut()
                .expect("a refreshed operation has explicit live state");
            live.health = source_health;
            live.freshness = freshness;
            if agent_work_is_active {
                live.last_outcome = None;
                live.error = None;
            } else if !preserve_recovery {
                live.last_outcome = Some(LensRefreshOutcome::Unchanged);
                live.error = None;
            }
        }
    }
}

fn refresh_projection_transition_is_valid(
    previous: &ProjectionRef,
    next: &ProjectionRef,
    outcome: LensContextRefreshOutcome,
) -> bool {
    match outcome {
        LensContextRefreshOutcome::Unchanged => next == previous,
        LensContextRefreshOutcome::Updated => {
            previous
                .revision
                .get()
                .checked_add(1)
                .is_some_and(|revision| revision == next.revision.get())
                && previous.digest != next.digest
        }
    }
}

fn validated_payload_map(
    payloads: Vec<LensMediaPayload>,
) -> Result<BTreeMap<String, LensMediaPayload>, String> {
    let mut next = BTreeMap::new();
    let mut uris = BTreeSet::new();
    for payload in payloads {
        if !uris.insert(payload.uri.clone()) {
            return Err("Lens media attachment URI is duplicated".into());
        }
        if next
            .insert(payload.attachment_id.clone(), payload)
            .is_some()
        {
            return Err("Lens media attachment identity is duplicated".into());
        }
    }
    Ok(next)
}

pub fn begin_agent_run(
    app: &AppHandle,
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
        if snapshot.lens.operation_id != Some(operation_id)
            || snapshot.lens.projection.as_ref() != Some(expected_projection)
            || &snapshot.config != expected_config
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

pub fn update_lens_state(
    app: &AppHandle,
    operation_id: Uuid,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    update_lens_state_if(
        app,
        |snapshot| snapshot.lens.operation_id == Some(operation_id),
        update,
    )
}

pub fn update_lens_state_for_context(
    app: &AppHandle,
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

pub fn update_lens_state_for_projection(
    app: &AppHandle,
    operation_id: Uuid,
    expected_projection: &ProjectionRef,
    expected_config: &AppConfig,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    update_lens_state_if(
        app,
        |snapshot| {
            &snapshot.config == expected_config
                && snapshot.lens.operation_id == Some(operation_id)
                && snapshot.lens.projection.as_ref() == Some(expected_projection)
        },
        update,
    )
}

pub fn update_lens_state_for_run(
    app: &AppHandle,
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

fn agent_run_has_authority(
    snapshot: &AppSnapshot,
    key: AgentRunKey,
    expected_config: &AppConfig,
) -> bool {
    &snapshot.config == expected_config
        && snapshot.lens.operation_id == Some(key.operation_id)
        && snapshot
            .lens
            .agent
            .as_ref()
            .is_some_and(|run| run.run_id == key.run_id)
}

fn update_lens_state_if(
    app: &AppHandle,
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

    fn projection_ref(revision: u64, digit: char) -> ProjectionRef {
        ProjectionRef::new(
            std::num::NonZeroU64::new(revision).expect("non-zero projection revision"),
            serde_json::from_str(&format!("\"{}\"", digit.to_string().repeat(64)))
                .expect("valid projection digest"),
        )
    }

    fn live_state(
        freshness: LensFreshness,
        last_outcome: Option<LensRefreshOutcome>,
        error: Option<&str>,
    ) -> crate::model::LensLiveState {
        crate::model::LensLiveState {
            lifecycle: LensMonitoringLifecycle::Watching,
            health: LensSourceHealth::Healthy,
            freshness,
            agent_refresh_interval_seconds: crate::model::LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
            last_outcome,
            error: error.map(str::to_string),
        }
    }

    fn representation(
        projection: ProjectionRef,
        context_revision: u64,
    ) -> crate::model::LensRepresentation {
        crate::model::LensRepresentation {
            representation_id: Uuid::from_u128(10),
            context_id: Uuid::nil(),
            context_revision,
            projection,
            run_id: Uuid::from_u128(11),
            output_blocks: vec![crate::model::LensOutputBlock::Markdown {
                message_id: None,
                text: "Settled representation".into(),
            }],
        }
    }

    fn context(context_id: Uuid, revision: u64) -> LensContext {
        LensContext {
            schema_version: crate::lens::LENS_CONTEXT_SCHEMA_VERSION,
            context_id,
            revision,
            sources: vec![],
            media: vec![],
            media_omissions: vec![],
            quality: crate::model::ExtractionQuality::Unavailable,
            diagnostics: vec![],
        }
    }

    fn target_set(operation_id: Uuid, title: &str) -> LensTargetSet {
        LensTargetSet::try_new(
            operation_id,
            vec![crate::model::SelectedWindow {
                identity: crate::model::WindowIdentity {
                    window_id: 42,
                    bundle_id: "example.browser".into(),
                    pid: 100,
                },
                facts: crate::model::WindowObservableFacts {
                    title: title.into(),
                    application_name: "Browser".into(),
                    frame: crate::model::Bounds {
                        x: 10.0,
                        y: 20.0,
                        width: 800.0,
                        height: 600.0,
                    },
                },
            }],
        )
        .expect("valid target set")
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
    fn active_run_authority_is_independent_from_the_latest_projection() {
        let operation_id = Uuid::from_u128(1);
        let run_id = Uuid::from_u128(2);
        let config = AppConfig::default();
        let projection = |revision, digit: char| {
            ProjectionRef::new(
                std::num::NonZeroU64::new(revision).expect("non-zero projection revision"),
                serde_json::from_str(&format!("\"{}\"", digit.to_string().repeat(64)))
                    .expect("valid projection digest"),
            )
        };
        let active_projection = projection(1, 'a');
        let latest_projection = projection(2, 'b');
        let mut snapshot = AppSnapshot::new(config.clone());
        snapshot.lens.operation_id = Some(operation_id);
        snapshot.lens.projection = Some(latest_projection);
        snapshot.lens.agent = Some(crate::model::AgentRunState {
            run_id,
            input_projection: Some(active_projection),
            kind: crate::model::AgentKind::Codex,
            adapter_name: "test-agent".into(),
            adapter_version: "1.0.0".into(),
            session_id: Some("session".into()),
            session_mode_id: Some("read-only".into()),
            auth_methods: vec![],
            received_updates: 0,
            stop_reason: None,
            authentication_message: None,
        });
        let key = AgentRunKey {
            operation_id,
            run_id,
        };

        assert!(agent_run_has_authority(&snapshot, key, &config));
        snapshot.lens.agent.as_mut().expect("active run").run_id = Uuid::from_u128(3);
        assert!(!agent_run_has_authority(&snapshot, key, &config));
    }

    #[test]
    fn picker_lease_rejects_reentry_and_releases_on_drop() {
        let control = PickerControl::default();
        let lease = control.try_begin().expect("first picker lease");
        assert!(control.try_begin().is_err());
        drop(lease);
        assert!(control.try_begin().is_ok());
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

    #[test]
    fn stale_refresh_has_no_authority_to_replace_current_observable_facts() {
        let operation_id = Uuid::from_u128(20);
        let context_id = Uuid::from_u128(21);
        let canonical_targets = target_set(operation_id, "Current title");
        let stale_targets = target_set(operation_id, "Stale worker title");
        let lens = LensState {
            operation_id: Some(operation_id),
            target_set: Some(canonical_targets.clone()),
            context: Some(context(context_id, 2)),
            live: Some(live_state(LensFreshness::Current, None, None)),
            ..LensState::default()
        };

        assert!(!lens_context_refresh_has_authority(
            &lens,
            operation_id,
            context_id,
            1,
        ));
        assert!(lens_context_refresh_has_authority(
            &lens,
            operation_id,
            context_id,
            2,
        ));
        assert_eq!(lens.target_set.as_ref(), Some(&canonical_targets));
        assert_ne!(lens.target_set.as_ref(), Some(&stale_targets));
    }

    #[test]
    fn unchanged_refresh_preserves_stale_recovery_state_and_representation_provenance() {
        let settled_projection = projection_ref(1, 'a');
        let latest_projection = projection_ref(2, 'b');
        let cases = [
            (
                LensStage::AuthenticationRequired,
                None,
                Some(LensRefreshOutcome::Failed),
                Some("authentication required"),
            ),
            (LensStage::Cancelled, None, None, None),
            (
                LensStage::Failed,
                Some("Agent failed"),
                Some(LensRefreshOutcome::Failed),
                Some("Agent failed"),
            ),
        ];

        for (stage, state_error, last_outcome, live_error) in cases {
            let mut lens = LensState {
                stage,
                projection: Some(latest_projection.clone()),
                representation: Some(representation(settled_projection.clone(), 4)),
                live: Some(live_state(LensFreshness::Stale, last_outcome, live_error)),
                error: state_error.map(str::to_string),
                ..LensState::default()
            };

            reconcile_lens_after_context_refresh(
                &mut lens,
                5,
                LensSourceHealth::Healthy,
                LensContextRefreshOutcome::Unchanged,
            );

            let retained = lens
                .representation
                .as_ref()
                .expect("retained representation");
            assert_eq!(retained.projection, settled_projection);
            assert_eq!(retained.context_revision, 4);
            assert_eq!(lens.stage, stage);
            assert_eq!(lens.error.as_deref(), state_error);
            let live = lens.live.as_ref().expect("live state");
            assert_eq!(live.freshness, LensFreshness::Stale);
            assert_eq!(live.last_outcome, last_outcome);
            assert_eq!(live.error.as_deref(), live_error);
        }
    }

    #[test]
    fn refresh_projection_transition_requires_exact_outcome_and_revision() {
        let first = projection_ref(1, 'a');
        let same = first.clone();
        let next = projection_ref(2, 'b');
        let skipped = projection_ref(3, 'c');
        let same_digest_next_revision = ProjectionRef::new(
            std::num::NonZeroU64::new(2).expect("non-zero projection revision"),
            first.digest.clone(),
        );

        assert!(refresh_projection_transition_is_valid(
            &first,
            &same,
            LensContextRefreshOutcome::Unchanged,
        ));
        assert!(refresh_projection_transition_is_valid(
            &first,
            &next,
            LensContextRefreshOutcome::Updated,
        ));
        assert!(!refresh_projection_transition_is_valid(
            &first,
            &next,
            LensContextRefreshOutcome::Unchanged,
        ));
        assert!(!refresh_projection_transition_is_valid(
            &first,
            &skipped,
            LensContextRefreshOutcome::Updated,
        ));
        assert!(!refresh_projection_transition_is_valid(
            &first,
            &same_digest_next_revision,
            LensContextRefreshOutcome::Updated,
        ));
    }

    #[test]
    fn unchanged_refresh_advances_only_matching_representation_provenance() {
        let projection = projection_ref(2, 'b');
        let mut lens = LensState {
            stage: LensStage::Completed,
            projection: Some(projection.clone()),
            representation: Some(representation(projection, 4)),
            live: Some(live_state(
                LensFreshness::Checking,
                Some(LensRefreshOutcome::Failed),
                Some("obsolete error"),
            )),
            error: Some("obsolete error".into()),
            ..LensState::default()
        };

        reconcile_lens_after_context_refresh(
            &mut lens,
            5,
            LensSourceHealth::Healthy,
            LensContextRefreshOutcome::Unchanged,
        );

        assert_eq!(
            lens.representation
                .as_ref()
                .expect("current representation")
                .context_revision,
            5
        );
        assert_eq!(lens.stage, LensStage::Completed);
        assert!(lens.error.is_none());
        let live = lens.live.as_ref().expect("live state");
        assert_eq!(live.freshness, LensFreshness::Current);
        assert_eq!(live.last_outcome, Some(LensRefreshOutcome::Unchanged));
        assert!(live.error.is_none());
    }

    #[test]
    fn updated_refresh_preserves_active_work_and_reopens_terminal_recovery() {
        for stage in [LensStage::Connecting, LensStage::Transforming] {
            let mut lens = LensState {
                stage,
                live: Some(live_state(
                    LensFreshness::None,
                    Some(LensRefreshOutcome::Updated),
                    None,
                )),
                ..LensState::default()
            };
            reconcile_lens_after_context_refresh(
                &mut lens,
                5,
                LensSourceHealth::Healthy,
                LensContextRefreshOutcome::Updated,
            );

            assert_eq!(lens.stage, stage);
            assert_eq!(
                lens.live.as_ref().map(|live| live.freshness),
                Some(LensFreshness::Checking)
            );
            assert_eq!(lens.live.as_ref().and_then(|live| live.last_outcome), None);
        }

        let settled_projection = projection_ref(1, 'a');
        let latest_projection = projection_ref(2, 'b');
        let mut failed = LensState {
            stage: LensStage::Failed,
            projection: Some(latest_projection),
            representation: Some(representation(settled_projection, 4)),
            live: Some(live_state(
                LensFreshness::Stale,
                Some(LensRefreshOutcome::Failed),
                Some("Agent failed"),
            )),
            error: Some("Agent failed".into()),
            ..LensState::default()
        };
        reconcile_lens_after_context_refresh(
            &mut failed,
            5,
            LensSourceHealth::Healthy,
            LensContextRefreshOutcome::Updated,
        );

        assert_eq!(failed.stage, LensStage::Ready);
        assert!(failed.error.is_none());
        let live = failed.live.as_ref().expect("live state");
        assert_eq!(live.freshness, LensFreshness::Stale);
        assert_eq!(live.last_outcome, None);
        assert!(live.error.is_none());
    }
}
