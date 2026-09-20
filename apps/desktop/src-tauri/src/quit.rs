//! User-requested quit admission. Native OS termination remains outside this gate.
use crate::app_state::AppState;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

pub(crate) const APPLICATION_QUIT_ID: &str = "quit_application";
const QUIT_BUTTON: &str = "Stop and Quit";

#[derive(Debug, Default, PartialEq, Eq)]
enum QuitState {
    #[default]
    Idle,
    Confirming(Uuid),
    Exiting,
}

#[derive(Debug, PartialEq, Eq)]
enum QuitAction {
    None,
    Confirm(Uuid),
    Exit,
}

impl QuitState {
    fn request(&mut self, has_work: bool, token: Uuid) -> QuitAction {
        if *self != Self::Idle {
            return QuitAction::None;
        }
        if has_work {
            *self = Self::Confirming(token);
            QuitAction::Confirm(token)
        } else {
            *self = Self::Exiting;
            QuitAction::Exit
        }
    }

    fn resolve(&mut self, token: Uuid, quit: bool) -> QuitAction {
        if *self != Self::Confirming(token) {
            return QuitAction::None;
        }
        if quit {
            *self = Self::Exiting;
            QuitAction::Exit
        } else {
            *self = Self::Idle;
            QuitAction::None
        }
    }
}

#[derive(Default)]
pub(crate) struct QuitCoordinator(Mutex<QuitState>);

pub(crate) fn has_pending_work(state: &AppState) -> Result<bool, String> {
    // Installation locks control runtime before this slot. Never retain the slot
    // while inspecting the control runtime in the reverse order.
    let controls = state
        .session_controls
        .lock()
        .map_err(|_| "Agent controls are unavailable".to_string())?
        .clone();
    let controls_busy = controls
        .map(|controls| controls.has_pending_work())
        .transpose()?
        .unwrap_or(false);
    Ok(controls_busy || state.agent_control.has_pending_work()?)
}

pub(crate) fn request<R: tauri::Runtime>(app: &AppHandle<R>) {
    let action = (|| -> Result<QuitAction, String> {
        let coordinator = app.state::<QuitCoordinator>();
        let mut state = coordinator
            .0
            .lock()
            .map_err(|_| "Quit state is unavailable".to_string())?;
        if *state != QuitState::Idle {
            return Ok(QuitAction::None);
        }
        let busy = app
            .try_state::<AppState>()
            .map(|state| has_pending_work(&state))
            .transpose()?
            .unwrap_or(false);
        Ok(state.request(busy, Uuid::new_v4()))
    })();
    match action {
        Ok(QuitAction::Exit) => app.exit(0),
        Ok(QuitAction::Confirm(token)) => {
            // Tauri invokes menu listeners while holding its listener mutex. Leave
            // that callback before entering a native modal loop, which may reenter it.
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let result = app.state::<crate::platform::Presentation<R>>().0.confirm_destructive_action(
                    &app,
                    "Stop the Agent and quit Lens?",
                    "The Agent is working or waiting for your response. Quitting will interrupt its work.",
                    QUIT_BUTTON,
                    "Cancel",
                );
                let coordinator = app.state::<QuitCoordinator>();
                let action = coordinator
                    .0
                    .lock()
                    .map(|mut state| state.resolve(token, matches!(result, Ok(true))));
                if matches!(action, Ok(QuitAction::Exit)) {
                    app.exit(0);
                } else if let Err(error) = result {
                    app.dialog()
                        .message(error.to_string())
                        .title("Unable to quit Lens")
                        .show(|_| {});
                }
            });
        }
        Ok(QuitAction::None) => {}
        Err(error) => {
            app.dialog()
                .message(error)
                .title("Unable to quit Lens")
                .show(|_| {});
        }
    }
}

/// OS termination and programmatic exits invalidate any outstanding callback.
pub(crate) fn mark_exiting<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Ok(mut state) = app.state::<QuitCoordinator>().0.lock() {
        *state = QuitState::Exiting;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_quit_and_confirmed_quit_are_terminal() {
        let token = Uuid::from_u128(1);
        let mut state = QuitState::default();
        assert_eq!(state.request(false, token), QuitAction::Exit);
        assert_eq!(state.request(true, token), QuitAction::None);
        let mut state = QuitState::default();
        assert_eq!(state.request(true, token), QuitAction::Confirm(token));
        assert_eq!(state.resolve(token, true), QuitAction::Exit);
        assert_eq!(state.resolve(token, true), QuitAction::None);
    }

    #[test]
    fn duplicate_requests_and_stale_callbacks_cannot_authorize_quit() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        let mut state = QuitState::default();
        assert_eq!(state.request(true, first), QuitAction::Confirm(first));
        assert_eq!(state.request(false, second), QuitAction::None);
        assert_eq!(state.resolve(second, true), QuitAction::None);
        assert_eq!(state.resolve(first, false), QuitAction::None);
        assert_eq!(state, QuitState::Idle);
        assert_eq!(state.request(true, second), QuitAction::Confirm(second));
        assert_eq!(state.resolve(first, true), QuitAction::None);
        state = QuitState::Exiting;
        assert_eq!(state.resolve(second, true), QuitAction::None);
    }

    #[test]
    fn completed_presentation_without_pending_agent_work_does_not_confirm() {
        let state = crate::test_support::state();
        state.runtime.write().unwrap().lens.stage = crate::model::LensStage::Completed;
        assert!(!has_pending_work(&state).unwrap());
    }
}
