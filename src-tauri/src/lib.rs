mod agent;
mod app_state;
mod commands;
mod model;
mod platform;
mod store;
mod ui;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let validate_a11y = std::env::var_os("PERSONAL_LENS_VALIDATE_A11Y").is_some();
    let validate_acp = std::env::var_os("PERSONAL_LENS_VALIDATE_ACP").is_some();
    let validation_agent = std::env::var("PERSONAL_LENS_VALIDATE_AGENT")
        .ok()
        .and_then(|value| match value.as_str() {
            "claude" => Some(model::AgentKind::Claude),
            "codex" => Some(model::AgentKind::Codex),
            _ => None,
        });
    let validation_cancel_after = std::env::var("PERSONAL_LENS_VALIDATE_CANCEL_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let validation_target = std::env::var("PERSONAL_LENS_VALIDATE_TARGET")
        .ok()
        .map(|json| {
            serde_json::from_str::<model::SelectedWindow>(&json)
                .expect("PERSONAL_LENS_VALIDATE_TARGET must be a SelectedWindow JSON object")
        });
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(app_state::AppState::load())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            ui::install_menu_bar(app)?;
            #[cfg(debug_assertions)]
            if std::env::var_os("PERSONAL_LENS_VALIDATE_UI").is_some() {
                ui::show_settings(app.handle())?;
                let validation_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if let Some(tray) = validation_handle.tray_by_id("personal-lens") {
                        match tray.rect() {
                            Ok(Some(rect)) => println!("PERSONAL_LENS_TRAY_RECT={rect:?}"),
                            Ok(None) => println!("PERSONAL_LENS_TRAY_RECT=unavailable"),
                            Err(error) => {
                                eprintln!("Unable to inspect tray icon bounds: {error}")
                            }
                        }
                    }
                });
            }
            if std::env::var_os("PERSONAL_LENS_VALIDATE_A11Y").is_none()
                && std::env::var_os("PERSONAL_LENS_VALIDATE_ACP").is_none()
            {
                let handle = app.handle().clone();
                let preferred_agent = handle
                    .state::<app_state::AppState>()
                    .config()
                    .map(|config| config.agent)
                    .unwrap_or(model::AgentKind::Claude);
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = agent::select_agent(handle, preferred_agent).await {
                        eprintln!("Unable to restore Agent selection: {error}");
                    }
                });
            }
            if !platform::accessibility_is_trusted() {
                platform::request_accessibility_trust();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::set_agent,
            commands::set_working_directory,
            commands::accessibility_permission,
            commands::request_accessibility_permission,
            commands::select_lens_target,
            commands::transform_lens,
            commands::authenticate_agent,
            commands::authenticate_agent_selection,
            commands::reauthenticate_agent_selection,
            commands::sign_out_agent_selection,
            commands::cancel_agent,
            commands::show_settings,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build PersonalLens")
        .run(move |app, event| match event {
            tauri::RunEvent::Ready if validate_a11y || validate_acp => {
                let handle = app.clone();
                let target = validation_target.clone();
                let agent = validation_agent;
                let cancel_after = validation_cancel_after;
                tauri::async_runtime::spawn(async move {
                    if let Some(agent) = agent {
                        if let Ok(mut snapshot) = handle
                            .state::<app_state::AppState>()
                            .runtime
                            .write()
                        {
                            if app_state::advance_revision(&mut snapshot).is_ok() {
                                snapshot.config.agent = agent;
                            }
                        }
                    }
                    let extraction_result = match target {
                        Some(target) => commands::extract_target(handle.clone(), target).await,
                        None => commands::select_and_extract(handle.clone()).await,
                    };
                    let result = match extraction_result {
                        Ok(state) if validate_acp && state.stage == model::LensStage::Ready => {
                            let operation_id = state
                                .operation_id
                                .expect("ready validation operation must have an identity");
                            if let Some(delay) = cancel_after {
                                let cancellation_handle = handle.clone();
                                tauri::async_runtime::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                                    let run_id = cancellation_handle
                                        .state::<app_state::AppState>()
                                        .lens()
                                        .ok()
                                        .filter(|lens| lens.operation_id == Some(operation_id))
                                        .and_then(|lens| lens.agent.map(|run| run.run_id));
                                    if let Some(run_id) = run_id {
                                        let _ = agent::cancel_current(
                                            &cancellation_handle,
                                            app_state::AgentRunKey {
                                                operation_id,
                                                run_id,
                                            },
                                        );
                                    }
                                });
                            }
                            agent::transform_current(handle.clone(), operation_id).await
                        }
                        other => other,
                    };
                    match result {
                        Ok(state) => {
                            let summary = serde_json::json!({
                                "stage": state.stage,
                                "target": state.target,
                                "quality": state.extraction.as_ref().map(|value| value.quality),
                                "metrics": state.extraction.as_ref().map(|value| &value.metrics),
                                "contains_offscreen_marker": state.input.as_ref().is_some_and(|input| {
                                    input.text.contains("PL_OFFSCREEN_END_MARKER_9F3A7C")
                                }),
                                "input_text_bytes": state.input.as_ref().map(|input| input.text.len()),
                                "agent": state.agent,
                                "transformed_text_bytes": state.transformed_text.as_ref().map(|text| text.len()),
                                "transformed_text": state.transformed_text,
                                "error": state.error,
                            });
                            match serde_json::to_string(&summary) {
                                Ok(json) if validate_acp => {
                                    println!("PERSONAL_LENS_E2E_RESULT={json}")
                                }
                                Ok(json) => println!("PERSONAL_LENS_A11Y_RESULT={json}"),
                            Err(error) => {
                                eprintln!("Unable to serialize A11y validation result: {error}")
                            }
                            }
                        }
                        Err(error) => eprintln!("PersonalLens A11y validation failed: {error}"),
                    }
                });
            }
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            _ => {}
        });
}
