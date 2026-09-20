//! Opt-in native quit validation with a synthetic run and isolated application data.
//! This module is absent from release builds and never launches an Agent provider.
use crate::app_state::AppState;
use tauri::{AppHandle, Manager};

pub(crate) fn start<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if !app.config().identifier.ends_with(".quit-validation") {
        return Err(
            "Quit validation requires an isolated .quit-validation application identifier".into(),
        );
    }
    let mode = std::env::var("LENS_VALIDATE_QUIT").unwrap_or_default();
    if !matches!(mode.as_str(), "idle" | "active") {
        return Err("LENS_VALIDATE_QUIT must be idle or active".into());
    }
    crate::ui::show_settings(app).map_err(|error| error.to_string())?;
    if mode == "active" {
        let run = app
            .state::<AppState>()
            .agent_control
            .begin_validation(uuid::Uuid::new_v4())?;
        tauri::async_runtime::spawn(async move {
            let mut cancellation = run.cancellation;
            println!("LENS_QUIT_FIXTURE=active");
            loop {
                tokio::select! {
                    result = cancellation.changed() => {
                        println!("LENS_QUIT_FIXTURE_CANCELLED={}", *cancellation.borrow());
                        if result.is_err() || *cancellation.borrow() { break; }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                        println!("LENS_QUIT_FIXTURE_HEARTBEAT=active");
                    }
                }
            }
        });
    } else {
        println!("LENS_QUIT_FIXTURE=idle");
    }
    Ok(())
}
