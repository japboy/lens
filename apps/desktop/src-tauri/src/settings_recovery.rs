//! Startup-only recovery: no application configuration or Agent runtime is admitted here.
use crate::store::{ConfigStore, RecoveryInfo};
use std::sync::Mutex;
use tauri::{Manager, Runtime};
use tauri_plugin_opener::OpenerExt;

pub(crate) const WINDOW_LABEL: &str = "settings-recovery";
pub(crate) struct RecoveryState {
    store: ConfigStore,
    error: Mutex<String>,
    mutation: Mutex<()>,
}

pub(crate) fn show<R: Runtime>(
    app: &tauri::App<R>,
    store: ConfigStore,
    error: String,
) -> Result<(), String> {
    app.manage(RecoveryState {
        store,
        error: Mutex::new(error),
        mutation: Mutex::new(()),
    });
    crate::ui::show_settings_recovery(app.handle()).map_err(|error| error.to_string())
}

pub(crate) fn command_allowed(label: &str, ready: bool, command: &str) -> bool {
    let recovery_command = matches!(
        command,
        "get_settings_recovery"
            | "open_recovery_settings_file"
            | "retry_settings_recovery"
            | "restore_recovery_prompt_presets"
    );
    if label == WINDOW_LABEL {
        !ready && recovery_command
    } else {
        ready && !recovery_command
    }
}

#[tauri::command]
pub(crate) fn get_settings_recovery(
    state: tauri::State<'_, RecoveryState>,
) -> Result<RecoveryInfo, String> {
    Ok(state.store.inspect_recovery(
        state
            .error
            .lock()
            .map_err(|_| "Recovery state is unavailable")?
            .clone(),
    ))
}

#[tauri::command]
pub(crate) fn open_recovery_settings_file<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, RecoveryState>,
) -> Result<(), String> {
    let info = get_settings_recovery(state)?;
    app.opener()
        .open_path(info.settings_path, None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn retry_settings_recovery<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, RecoveryState>,
) -> Result<(), String> {
    let _guard = state
        .mutation
        .lock()
        .map_err(|_| "Recovery is already in progress")?;
    match state.store.try_load() {
        Ok(_) => app.restart(),
        Err(error) => {
            *state
                .error
                .lock()
                .map_err(|_| "Recovery state is unavailable")? = error.clone();
            Err(error)
        }
    }
}

#[tauri::command]
pub(crate) fn restore_recovery_prompt_presets<R: Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, RecoveryState>,
    expected_digest: String,
) -> Result<(), String> {
    let _guard = state
        .mutation
        .lock()
        .map_err(|_| "Recovery is already in progress")?;
    state.store.restore_prompt_presets(&expected_digest)?;
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_is_disjoint_before_and_after_startup() {
        for command in [
            "get_app_snapshot",
            "set_agent",
            "update_prompt_presets",
            "select_lens_target",
            "show_settings",
        ] {
            assert!(!command_allowed(WINDOW_LABEL, false, command));
            assert!(!command_allowed("settings", false, command));
            assert!(!command_allowed(WINDOW_LABEL, true, command));
            assert!(command_allowed("settings", true, command));
        }
        assert!(command_allowed(
            WINDOW_LABEL,
            false,
            "get_settings_recovery"
        ));
        assert!(!command_allowed("settings", true, "get_settings_recovery"));
    }
}
