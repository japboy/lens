use crate::{
    lens::LensMediaPayload,
    model::{AgentRuntimeState, AgentSelectionState, AppConfig, AppSnapshot, LensStage, LensState},
    store::ConfigStore,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{watch, Mutex as AsyncMutex};
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

struct ActiveAgentRun {
    key: AgentRunKey,
    cancellation: watch::Sender<bool>,
}

#[derive(Default)]
pub struct AgentControl {
    active: Mutex<Option<ActiveAgentRun>>,
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
        let active = self
            .active
            .lock()
            .map_err(|_| "agent control lock is poisoned".to_string())?;
        if let Some(active) = active.as_ref() {
            let _ = active.cancellation.send(true);
            Ok(Some(active.key))
        } else {
            Ok(None)
        }
    }

    pub fn cancel(&self, key: AgentRunKey) -> Result<bool, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "agent control lock is poisoned".to_string())?;
        let Some(active) = active.as_ref().filter(|active| active.key == key) else {
            return Ok(false);
        };
        let _ = active.cancellation.send(true);
        Ok(true)
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
    pub(crate) runtime: RwLock<AppSnapshot>,
    pub agent_runtime_install: AsyncMutex<()>,
    pub agent_control: AgentControl,
    pub lens_media: LensMediaStore,
    pub picker_control: PickerControl,
    pub store: ConfigStore,
}

impl AppState {
    pub fn load() -> Self {
        let store = ConfigStore::new();
        let config = store.load();
        Self {
            runtime: RwLock::new(AppSnapshot::new(config)),
            agent_runtime_install: AsyncMutex::new(()),
            agent_control: AgentControl::default(),
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

pub fn begin_agent_run(
    app: &AppHandle,
    operation_id: Uuid,
    initialize: impl FnOnce(Uuid, &mut LensState),
) -> Result<AgentRunHandle, String> {
    let state = app.state::<AppState>();
    let (run, snapshot) = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        if snapshot.lens.operation_id != Some(operation_id) {
            return Err("Lens operation was superseded before the Agent run started".into());
        }
        let revision = next_revision(&snapshot)?;
        let run = state.agent_control.begin(operation_id)?;
        initialize(run.key.run_id, &mut snapshot.lens);
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
    update_lens_state_if(app, |lens| lens.operation_id == Some(operation_id), update)
}

pub fn update_lens_state_for_run(
    app: &AppHandle,
    key: AgentRunKey,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    update_lens_state_if(
        app,
        |lens| {
            lens.operation_id == Some(key.operation_id)
                && lens
                    .agent
                    .as_ref()
                    .is_some_and(|run| run.run_id == key.run_id)
        },
        update,
    )
}

fn update_lens_state_if(
    app: &AppHandle,
    predicate: impl FnOnce(&LensState) -> bool,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let (snapshot, sync_tray) = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        if !predicate(&snapshot.lens) {
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
            uri: "personallens://fixture/media-node-1".into(),
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
            uri: "personallens://fixture/media-node-1".into(),
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
}
