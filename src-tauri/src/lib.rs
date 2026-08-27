mod agent;
mod agent_runtime;
mod app_state;
mod commands;
mod model;
mod platform;
mod store;
mod ui;

#[cfg(debug_assertions)]
use base64::prelude::*;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let validate_a11y = std::env::var_os("PERSONAL_LENS_VALIDATE_A11Y").is_some();
    let validate_acp = std::env::var_os("PERSONAL_LENS_VALIDATE_ACP").is_some();
    let validate_rich_output =
        cfg!(debug_assertions) && std::env::var_os("PERSONAL_LENS_VALIDATE_RICH_OUTPUT").is_some();
    let validation_runtime = std::env::var("PERSONAL_LENS_VALIDATE_RUNTIME")
        .ok()
        .map(|value| match value.as_str() {
            "claude" => model::AgentKind::Claude,
            "codex" => model::AgentKind::Codex,
            _ => panic!("PERSONAL_LENS_VALIDATE_RUNTIME must be claude or codex"),
        });
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
        .setup(move |app| {
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
                && std::env::var_os("PERSONAL_LENS_VALIDATE_RUNTIME").is_none()
                && !validate_rich_output
            {
                let handle = app.handle().clone();
                let preferred_agent = handle
                    .state::<app_state::AppState>()
                    .config()
                    .map(|config| config.agent)
                    .unwrap_or(model::AgentKind::Claude);
                tauri::async_runtime::spawn(async move {
                    if let Err(error) =
                        agent::restore_agent_selection(handle, preferred_agent).await
                    {
                        eprintln!("Unable to restore Agent selection: {error}");
                    }
                });
            }
            if !validate_rich_output && !platform::accessibility_is_trusted() {
                platform::request_accessibility_trust();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::set_agent,
            commands::set_working_directory,
            commands::set_response_prompt,
            commands::reset_response_prompt,
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
            tauri::RunEvent::Ready if validation_runtime.is_some() => {
                let handle = app.clone();
                let agent = validation_runtime.expect("validated above");
                tauri::async_runtime::spawn(async move {
                    let (summary, exit_code) = match agent_runtime::resolve(&handle, agent).await {
                        Ok(runtime) => (
                            serde_json::json!({
                                "status": "ready",
                                "agent": runtime.kind,
                                "adapter_name": runtime.adapter_name,
                                "adapter_version": runtime.adapter_version,
                                "safe_mode_id": runtime.safe_mode_id,
                            }),
                            0,
                        ),
                        Err(error) => (
                            serde_json::json!({
                                "status": "failed",
                                "agent": agent,
                                "error": error,
                            }),
                            1,
                        ),
                    };
                    println!("PERSONAL_LENS_RUNTIME_RESULT={summary}");
                    handle.exit(exit_code);
                });
            }
            #[cfg(debug_assertions)]
            tauri::RunEvent::Ready if validate_rich_output => {
                if let Err(error) = show_rich_output_validation(app.clone()) {
                    eprintln!("Unable to show rich output validation: {error}");
                }
            }
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
                            let transformed_text = state
                                .output_blocks
                                .iter()
                                .filter_map(|block| match block {
                                    model::LensOutputBlock::Markdown { text, .. } => Some(text.as_str()),
                                    _ => None,
                                })
                                .collect::<String>();
                            let output_blocks = state
                                .output_blocks
                                .iter()
                                .map(|block| match block {
                                    model::LensOutputBlock::Markdown {
                                        message_id,
                                        text,
                                    } => serde_json::json!({
                                        "type": "markdown",
                                        "message_id": message_id,
                                        "text_bytes": text.len(),
                                    }),
                                    model::LensOutputBlock::Image {
                                        message_id,
                                        mime_type,
                                        data,
                                        uri,
                                    } => serde_json::json!({
                                        "type": "image",
                                        "message_id": message_id,
                                        "mime_type": mime_type,
                                        "data_bytes": data.len(),
                                        "uri": uri,
                                    }),
                                    model::LensOutputBlock::Unsupported {
                                        message_id,
                                        content_type,
                                    } => serde_json::json!({
                                        "type": "unsupported",
                                        "message_id": message_id,
                                        "content_type": content_type,
                                    }),
                                })
                                .collect::<Vec<_>>();
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
                                "transformed_text_bytes": transformed_text.len(),
                                "transformed_text": transformed_text,
                                "output_blocks": output_blocks,
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
            tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } => {
                api.prevent_exit();
            }
            _ => {}
        });
}

#[cfg(debug_assertions)]
fn show_rich_output_validation(app: tauri::AppHandle) -> Result<(), String> {
    let operation_id = uuid::Uuid::new_v4();
    let target = model::SelectedWindow {
        window_id: 0,
        title: "Rich output validation".into(),
        application_name: "PersonalLens Fixture".into(),
        bundle_id: "com.github.japboy.personallens.fixture".into(),
        pid: std::process::id() as i32,
        frame: model::Bounds {
            x: 120.0,
            y: 100.0,
            width: 1_200.0,
            height: 800.0,
        },
    };
    let state = model::LensState {
        operation_id: Some(operation_id),
        stage: model::LensStage::Completed,
        target: Some(target.clone()),
        output_blocks: vec![
            model::LensOutputBlock::Markdown {
                message_id: Some("validation-message".into()),
                text: "# Rich output validation\n\nThis Markdown was emitted before the ACP image block."
                    .into(),
            },
            model::LensOutputBlock::Image {
                message_id: Some("validation-message".into()),
                mime_type: "image/png".into(),
                data: BASE64_STANDARD.encode(include_bytes!("../icons/128x128@2x.png")),
                uri: Some("fixture://personal-lens-icon".into()),
            },
            model::LensOutputBlock::Markdown {
                message_id: Some("validation-message".into()),
                text: "The image block rendered above; this Markdown block follows it.".into(),
            },
        ],
        ..model::LensState::default()
    };
    app_state::publish_lens_state(&app, state)?;
    ui::show_overlay(&app, &target, operation_id).map_err(|error| error.to_string())?;
    println!("PERSONAL_LENS_RICH_OUTPUT_RESULT=displayed");
    Ok(())
}
