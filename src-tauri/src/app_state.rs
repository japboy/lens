use crate::{
    model::{AgentSelectionState, AppConfig, LensState},
    store::ConfigStore,
};
use std::sync::{Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;
use uuid::Uuid;

struct ActiveAgentRun {
    operation_id: Uuid,
    cancellation: watch::Sender<bool>,
}

#[derive(Default)]
pub struct AgentControl {
    active: Mutex<Option<ActiveAgentRun>>,
}

impl AgentControl {
    pub fn begin(&self, operation_id: Uuid) -> Result<watch::Receiver<bool>, String> {
        let (cancellation, receiver) = watch::channel(false);
        let mut active = self
            .active
            .lock()
            .map_err(|_| "agent control lock is poisoned".to_string())?;
        if let Some(previous) = active.replace(ActiveAgentRun {
            operation_id,
            cancellation,
        }) {
            let _ = previous.cancellation.send(true);
        }
        Ok(receiver)
    }

    pub fn cancel(&self) -> Result<Option<Uuid>, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "agent control lock is poisoned".to_string())?;
        if let Some(active) = active.as_ref() {
            let _ = active.cancellation.send(true);
            Ok(Some(active.operation_id))
        } else {
            Ok(None)
        }
    }

    pub fn finish(&self, operation_id: Uuid) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "agent control lock is poisoned".to_string())?;
        if active
            .as_ref()
            .is_some_and(|run| run.operation_id == operation_id)
        {
            *active = None;
        }
        Ok(())
    }
}

pub struct AppState {
    pub config: RwLock<AppConfig>,
    pub agent_selection: RwLock<AgentSelectionState>,
    pub lens: RwLock<LensState>,
    pub agent_control: AgentControl,
    pub store: ConfigStore,
}

impl AppState {
    pub fn load() -> Self {
        let store = ConfigStore::new();
        let config = store.load();
        Self {
            config: RwLock::new(config),
            agent_selection: RwLock::new(AgentSelectionState::default()),
            lens: RwLock::new(LensState::default()),
            agent_control: AgentControl::default(),
            store,
        }
    }
}

pub fn publish_agent_selection(app: &AppHandle, next: AgentSelectionState) -> Result<(), String> {
    let state = app.state::<AppState>();
    {
        let mut guard = state
            .agent_selection
            .write()
            .map_err(|_| "agent selection state lock is poisoned".to_string())?;
        *guard = next.clone();
    }
    crate::ui::sync_tray_menu(app)?;
    app.emit("agent-selection-changed", next)
        .map_err(|error| error.to_string())
}

pub fn update_agent_selection(
    app: &AppHandle,
    operation_id: Uuid,
    update: impl FnOnce(&mut AgentSelectionState),
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let next = {
        let mut guard = state
            .agent_selection
            .write()
            .map_err(|_| "agent selection state lock is poisoned".to_string())?;
        if guard.operation_id != Some(operation_id) {
            return Ok(false);
        }
        update(&mut guard);
        guard.clone()
    };
    crate::ui::sync_tray_menu(app)?;
    app.emit("agent-selection-changed", next)
        .map_err(|error| error.to_string())?;
    Ok(true)
}

pub fn publish_lens_state(app: &AppHandle, next: LensState) -> Result<(), String> {
    let state = app.state::<AppState>();
    {
        let mut guard = state
            .lens
            .write()
            .map_err(|_| "lens state lock is poisoned".to_string())?;
        *guard = next.clone();
    }
    app.emit("lens-state-changed", next)
        .map_err(|error| error.to_string())
}

pub fn update_lens_state(
    app: &AppHandle,
    operation_id: Uuid,
    update: impl FnOnce(&mut LensState),
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let next = {
        let mut guard = state
            .lens
            .write()
            .map_err(|_| "lens state lock is poisoned".to_string())?;
        if guard.operation_id != Some(operation_id) {
            return Ok(false);
        }
        update(&mut guard);
        guard.clone()
    };
    app.emit("lens-state-changed", next)
        .map_err(|error| error.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_agent_run_cancels_and_outlives_the_previous_run() {
        let control = AgentControl::default();
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        let mut first = control.begin(first_id).expect("first run");
        let mut second = control.begin(second_id).expect("second run");

        assert!(*first.borrow_and_update());

        control.finish(first_id).expect("finish stale run");
        assert_eq!(
            control.cancel().expect("cancel active run"),
            Some(second_id)
        );
        assert!(second.has_changed().expect("second cancellation change"));
        assert!(*second.borrow_and_update());
    }
}
