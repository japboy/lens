mod agent;
mod agent_output;
mod agent_preferences;
mod agent_runtime;
mod app_state;
mod commands;
use use_case::confirm_targets;
#[cfg(test)]
mod contract_tests;
use use_case::elicitation;
mod live_runtime;
pub use use_case::live_sync;
#[cfg(test)]
mod confirmation_tests;
mod media_protocol;
mod model;
mod platform;
mod session_controls;
#[cfg(test)]
mod shell_tests;
mod store;
#[cfg(test)]
mod test_support;
mod ui;

#[cfg(debug_assertions)]
use base64::prelude::*;
use domain::{lens, prompt_template};
use tauri::Manager;

/// Generate product assets, capabilities and embedded metadata once for every runtime.
fn product_context<R: tauri::Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

fn configure_shell<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    state: app_state::AppState,
    presentation: platform::Presentation<R>,
    tray: ui::TrayPresentation<R>,
    agents: agent::AgentServices<R>,
) -> tauri::Builder<R> {
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .manage(presentation)
        .manage(tray)
        .manage(agents)
        .manage(ui::LensWindowPresentationState::default())
        .register_uri_scheme_protocol(media_protocol::LENS_MEDIA_SCHEME, media_protocol::handle)
        .invoke_handler(command_handler())
}

/// One command registration for native composition and common-shell IPC verification.
fn command_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::get_app_snapshot,
        commands::set_agent,
        commands::set_working_directory,
        commands::set_agent_prompt_template,
        commands::reset_agent_prompt_template,
        commands::accessibility_permission,
        commands::request_accessibility_permission,
        commands::select_lens_target,
        commands::add_lens_target,
        commands::remove_lens_target,
        commands::confirm_lens_targets,
        commands::retry_lens_transform,
        commands::pause_lens,
        commands::resume_lens,
        commands::stop_lens,
        commands::authenticate_agent,
        commands::authenticate_agent_selection,
        commands::reauthenticate_agent_selection,
        commands::sign_out_agent_selection,
        commands::cancel_agent,
        commands::set_session_option,
        commands::set_agent_defaults,
        commands::preview_agent_model,
        commands::respond_agent_interaction,
        commands::show_settings,
    ]
}

#[cfg(target_os = "macos")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    run_with_runtime(
        tauri::Builder::default(),
        platform::macos_services(),
        platform::macos_presentation(),
    );
}

/// Shared shell implementation. Product entry points still admit only supported native targets.
pub fn run_with_runtime<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    services: platform::Services,
    presentation: platform::Presentation<R>,
) {
    let validate_interactions =
        cfg!(debug_assertions) && std::env::var_os("LENS_VALIDATE_INTERACTIONS").is_some();
    let validate_a11y = std::env::var_os("LENS_VALIDATE_A11Y").is_some();
    let validate_acp = std::env::var_os("LENS_VALIDATE_ACP").is_some();
    let validate_rich_output =
        cfg!(debug_assertions) && std::env::var_os("LENS_VALIDATE_RICH_OUTPUT").is_some();
    let validation_runtime =
        std::env::var("LENS_VALIDATE_RUNTIME")
            .ok()
            .map(|value| match value.as_str() {
                "claude" => model::AgentKind::Claude,
                "codex" => model::AgentKind::Codex,
                _ => panic!("LENS_VALIDATE_RUNTIME must be claude or codex"),
            });
    let validation_agent = std::env::var("LENS_VALIDATE_AGENT")
        .ok()
        .and_then(|value| match value.as_str() {
            "claude" => Some(model::AgentKind::Claude),
            "codex" => Some(model::AgentKind::Codex),
            _ => None,
        });
    let validation_cancel_after = std::env::var("LENS_VALIDATE_CANCEL_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let validation_target = std::env::var("LENS_VALIDATE_TARGET").ok().map(|json| {
        serde_json::from_str::<model::SelectedWindow>(&json)
            .expect("LENS_VALIDATE_TARGET must be a SelectedWindow JSON object")
    });
    configure_shell(
        builder,
        app_state::AppState::load(services),
        presentation,
        ui::TrayPresentation(std::sync::Arc::new(ui::NativeTrayOutput)),
        agent::AgentServices(std::sync::Arc::new(agent::ManagedAgentHost)),
    )
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(if validate_a11y {
                tauri::ActivationPolicy::Regular
            } else {
                tauri::ActivationPolicy::Accessory
            });
            ui::install_menu_bar(app)?;
            #[cfg(debug_assertions)]
            if std::env::var_os("LENS_VALIDATE_UI").is_some() {
                ui::show_settings(app.handle())?;
                let validation_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    if let Some(tray) = validation_handle.tray_by_id("lens") {
                        match tray.rect() {
                            Ok(Some(rect)) => println!("LENS_TRAY_RECT={rect:?}"),
                            Ok(None) => println!("LENS_TRAY_RECT=unavailable"),
                            Err(error) => {
                                eprintln!("Unable to inspect tray icon bounds: {error}")
                            }
                        }
                    }
                });
            }
            if std::env::var_os("LENS_VALIDATE_A11Y").is_none()
                && std::env::var_os("LENS_VALIDATE_ACP").is_none()
                && std::env::var_os("LENS_VALIDATE_RUNTIME").is_none()
                && !validate_rich_output
                && !validate_interactions
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
            let state = app.state::<app_state::AppState>();
            if !validate_rich_output && !state.platform.trust.inspect() {
                state.platform.trust.request();
            }
            Ok(())
        })
        .build(product_context())
        .expect("failed to build Lens")
        .run(move |app, event| match event {
            #[cfg(debug_assertions)]
            tauri::RunEvent::Ready if validate_interactions => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let result = tokio::time::timeout(std::time::Duration::from_secs(240), session_controls::validation::run(&app)).await;
                    let passed = matches!(result, Ok(Ok(())));
                    println!("LENS_INTERACTION_RESULT={}", serde_json::json!({"passed":passed,"error":format!("{result:?}")}));
                    app.exit(if passed {0} else {1});
                });
            }
            tauri::RunEvent::ExitRequested { code: Some(_), .. } | tauri::RunEvent::Exit => {
                session_controls::close_active(app);
                let _ = app.state::<app_state::AppState>().agent_control.cancel_active();
            }
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
                    println!("LENS_RUNTIME_RESULT={summary}");
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
                            let authoritative_output = state
                                .representation
                                .as_ref()
                                .map(|representation| representation.output_blocks.as_slice())
                                .unwrap_or(state.output_blocks.as_slice());
                            let transformed_text = authoritative_output
                                .iter()
                                .filter_map(|block| match block {
                                    model::LensOutputBlock::Markdown { text, .. } => Some(text.as_str()),
                                    _ => None,
                                })
                                .collect::<String>();
                            let output_blocks = authoritative_output.iter()
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
                                "target_set": state.target_set,
                                "quality": state.context.as_ref().map(|value| value.quality),
                                "accessibility_metrics": state.context.as_ref().map(|context| {
                                    context.sources.iter().map(|source| serde_json::json!({
                                        "target_id": source.target_id,
                                        "metrics": source.capture.metrics,
                                    })).collect::<Vec<_>>()
                                }),
                                "media_capture": state.context.as_ref().map(|context| {
                                    serde_json::json!({
                                        "attachment_count": context.media.len(),
                                        "attachments": context.media.iter().map(|attachment| serde_json::json!({
                                            "id": attachment.id,
                                            "scope": attachment.scope,
                                            "source_node_id": attachment.source_node_id,
                                            "pixel_width": attachment.pixel_width,
                                            "pixel_height": attachment.pixel_height,
                                            "encoded_bytes": attachment.encoded_bytes,
                                        })).collect::<Vec<_>>(),
                                        "encoded_bytes": context.media.iter()
                                            .map(|attachment| attachment.encoded_bytes)
                                            .sum::<usize>(),
                                        "omission_count": context.media_omissions.iter()
                                            .map(|omission| omission.omitted_count)
                                            .sum::<usize>(),
                                        "omission_group_count": context.media_omissions.len(),
                                        "omissions": context.media_omissions,
                                    })
                                }),
                                "contains_offscreen_marker": state.input.as_ref().is_some_and(|input| {
                                    input.contains_text("LENS_OFFSCREEN_END_MARKER_9F3A7C")
                                }),
                                "input_context_bytes": state.input.as_ref().and_then(|input| {
                                    input.serialized_len().ok()
                                }),
                                "agent": state.agent,
                                "transformed_text_bytes": transformed_text.len(),
                                "transformed_text": transformed_text,
                                "output_blocks": output_blocks,
                                "error": state.error,
                            });
                            match serde_json::to_string(&summary) {
                                Ok(json) if validate_acp => {
                                    println!("LENS_E2E_RESULT={json}")
                                }
                                Ok(json) => println!("LENS_A11Y_RESULT={json}"),
                                Err(error) => {
                                    eprintln!("Unable to serialize A11y validation result: {error}")
                                }
                            }
                        }
                        Err(error) => eprintln!("Lens A11y validation failed: {error}"),
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
fn show_rich_output_validation<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<(), String> {
    let operation_id = uuid::Uuid::new_v4();
    let first_input_image = include_bytes!("../icons/128x128@2x.png");
    let second_input_image = include_bytes!("../icons/128x128.png");
    let target = model::SelectedWindow {
        identity: model::WindowIdentity {
            window_id: 0,
            bundle_id: "com.github.japboy.lens.fixture".into(),
            pid: std::process::id() as i32,
        },
        facts: model::WindowObservableFacts {
            title: "Rich output validation".into(),
            application_name: "Lens Fixture".into(),
            frame: model::Bounds {
                x: 120.0,
                y: 100.0,
                width: 1_200.0,
                height: 800.0,
            },
        },
    };
    let target_id = lens::target_id(&target);
    let target_set = lens::LensTargetSet::try_new(operation_id, vec![target.clone()])
        .map_err(|error| error.to_string())?;
    let first_attachment_id = "media-node-000001".to_string();
    let first_media_uri = format!("lens://context/{operation_id}/1/media/{first_attachment_id}");
    let first_attachment = lens::LensMediaAttachment {
        id: first_attachment_id.clone(),
        target_id: target_id.clone(),
        uri: first_media_uri.clone(),
        scope: lens::LensMediaScope::AxElementRegion,
        source_node_id: Some("node-000001".into()),
        source_bounds: model::Bounds {
            x: 240.0,
            y: 180.0,
            width: 256.0,
            height: 256.0,
        },
        captured_bounds: model::Bounds {
            x: 240.0,
            y: 180.0,
            width: 256.0,
            height: 256.0,
        },
        coverage: lens::LensMediaCoverage::FullRegion,
        coordinate_space: lens::LensCoordinateSpace::ScreenPoints,
        mime_type: "image/png".into(),
        pixel_width: 256,
        pixel_height: 256,
        encoded_bytes: first_input_image.len(),
    };
    let second_attachment_id = "media-node-000002".to_string();
    let second_media_uri = format!("lens://context/{operation_id}/1/media/{second_attachment_id}");
    let second_attachment = lens::LensMediaAttachment {
        id: second_attachment_id.clone(),
        target_id: target_id.clone(),
        uri: second_media_uri.clone(),
        scope: lens::LensMediaScope::AxElementRegion,
        source_node_id: Some("node-000002".into()),
        source_bounds: model::Bounds {
            x: 540.0,
            y: 140.0,
            width: 256.0,
            height: 192.0,
        },
        captured_bounds: model::Bounds {
            x: 540.0,
            y: 180.0,
            width: 128.0,
            height: 128.0,
        },
        coverage: lens::LensMediaCoverage::VisibleSubregion,
        coordinate_space: lens::LensCoordinateSpace::ScreenPoints,
        mime_type: "image/png".into(),
        pixel_width: 128,
        pixel_height: 128,
        encoded_bytes: second_input_image.len(),
    };
    let source = lens::LensSource::from(&target_set.targets[0]);
    let context = lens::LensContext {
        schema_version: lens::LENS_CONTEXT_SCHEMA_VERSION,
        context_id: operation_id,
        revision: 1,
        sources: vec![lens::LensAccessibilitySource {
            source_id: format!(
                "macos:{}:{}:accessibility",
                target.identity.bundle_id, target.identity.window_id
            ),
            target_id: target_id.clone(),
            revision: 1,
            source: source.clone(),
            capture: model::ExtractionResult {
                quality: model::ExtractionQuality::Full,
                resolved_window: None,
                nodes: Vec::new(),
                text: "Rich output validation fixture".into(),
                metrics: model::ExtractionMetrics {
                    visited_nodes: 48,
                    text_bytes: 1_024,
                    offscreen_text_nodes: 6,
                    resource_ref_count: 2,
                    resource_uri_bytes: 84,
                    ..model::ExtractionMetrics::default()
                },
                diagnostics: Vec::new(),
            },
            document: None,
            quality: model::ExtractionQuality::Full,
        }],
        media: vec![first_attachment.clone(), second_attachment.clone()],
        media_omissions: Vec::new(),
        quality: model::ExtractionQuality::Full,
        diagnostics: Vec::new(),
    };
    let input = lens::LensInput {
        schema_version: lens::LENS_INPUT_SCHEMA_VERSION,
        context_id: operation_id,
        context_revision: 1,
        sources: vec![lens::LensInputSource {
            source_id: format!(
                "macos:{}:{}:accessibility",
                target.identity.bundle_id, target.identity.window_id
            ),
            target_id,
            source_revision: 1,
            source,
            document: Some(lens::LensDocumentProjection {
                nodes: vec![
                    lens::LensContentNode {
                        id: "node-000001".into(),
                        parent_id: None,
                        kind: lens::LensNodeKind::Image,
                        role: Some("AXImage".into()),
                        subrole: None,
                        title: Some("Lens validation icon".into()),
                        value: None,
                        description: Some("First input media preview fixture".into()),
                        media_refs: vec![first_attachment_id.clone()],
                        resource_refs: vec![model::ResourceReference {
                            uri: "https://example.test/assets/validation-icon.png".into(),
                            source_attribute: "AXURL".into(),
                        }],
                    },
                    lens::LensContentNode {
                        id: "node-000002".into(),
                        parent_id: None,
                        kind: lens::LensNodeKind::Image,
                        role: Some("AXImage".into()),
                        subrole: None,
                        title: Some("Lens validation icon thumbnail".into()),
                        value: None,
                        description: Some("Second input media preview fixture".into()),
                        media_refs: vec![second_attachment_id.clone()],
                        resource_refs: vec![model::ResourceReference {
                            uri: "blob:https://example.test/fixture-image".into(),
                            source_attribute: "AXURL".into(),
                        }],
                    },
                ],
            }),
            quality: model::ExtractionQuality::Full,
            omissions: Vec::new(),
        }],
        media: vec![first_attachment, second_attachment],
        media_omissions: Vec::new(),
        quality: model::ExtractionQuality::Full,
    };
    let app_state = app.state::<app_state::AppState>();
    app_state.lens_media.begin(operation_id)?;
    if !app_state.lens_media.replace(
        operation_id,
        vec![
            lens::LensMediaPayload {
                attachment_id: first_attachment_id,
                uri: first_media_uri,
                mime_type: "image/png".into(),
                data: BASE64_STANDARD.encode(first_input_image),
            },
            lens::LensMediaPayload {
                attachment_id: second_attachment_id,
                uri: second_media_uri,
                mime_type: "image/png".into(),
                data: BASE64_STANDARD.encode(second_input_image),
            },
        ],
    )? {
        return Err("rich-output validation media operation was superseded".into());
    }
    let completed_state = model::LensState {
        operation_id: Some(operation_id),
        stage: model::LensStage::Completed,
        target_set: Some(target_set.clone()),
        context: Some(context),
        input: Some(input),
        output_blocks: rich_output_validation_blocks()?,
        ..model::LensState::default()
    };
    let mut progress_state = completed_state.clone();
    progress_state.stage = model::LensStage::Transforming;
    app_state::publish_lens_state(&app, progress_state)?;
    ui::show_lens_window(&app, &target_set).map_err(|error| error.to_string())?;
    let validation_app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Some(window) = validation_app.get_webview_window(ui::LENS_WINDOW_LABEL) {
            let script = r#"
          (() => {
            const overlayRoot = () => document
              .querySelector('lens-app')
              ?.shadowRoot
              ?.querySelector('lens-overlay-view')
              ?.shadowRoot;
            const root = overlayRoot();
            const snackbar = root?.querySelector('.lens-progress-snackbar');
            const progressRegion = root?.querySelector('.lens-progress-region');
            const footer = root?.querySelector('.overlay-footer');
            const snackbarBounds = snackbar?.getBoundingClientRect();
            const footerBounds = footer?.getBoundingClientRect();
            const appIcon = root?.querySelector('.overlay-app-icon');
            console.warn('LENS_SNACKBAR_RESULT=' + JSON.stringify({
              displayed: Boolean(snackbar),
              role: progressRegion?.getAttribute('role') ?? null,
              text: snackbar?.textContent?.replace(/\s+/g, ' ').trim() ?? null,
              region_parent: progressRegion?.parentElement?.className ?? null,
              main_contains_region: Boolean(root?.querySelector('.overlay-main .lens-progress-region')),
              snackbar_bottom: snackbarBounds?.bottom ?? null,
              footer_top: footerBounds?.top ?? null,
              footer_overflow: footer ? getComputedStyle(footer).overflow : null,
              app_icon_displayed: Boolean(appIcon?.complete && appIcon?.naturalWidth > 0),
              app_icon_source: appIcon?.getAttribute('src') ?? null,
              visible_app_title: Boolean(root?.querySelector('.overlay-title:not(.visually-hidden)'))
            }));
            root?.querySelector('#source-tab')?.click();
            window.setTimeout(() => {
              const currentRoot = overlayRoot();
              const thumbnails = currentRoot?.querySelectorAll('.input-media-thumbnail') ?? [];
              thumbnails[1]?.click();
              window.setTimeout(() => {
              const selectedRoot = overlayRoot();
              const image = selectedRoot?.querySelector('.input-media-preview figure > img');
              const source = currentRoot?.querySelector('.source-content code')?.textContent ?? '';
              console.warn('LENS_SOURCE_PREVIEW_RESULT=' + JSON.stringify({
                displayed: Boolean(image?.complete && image?.naturalWidth > 0),
                natural_width: image?.naturalWidth ?? 0,
                natural_height: image?.naturalHeight ?? 0,
                uri: image?.getAttribute('src') ?? null,
                thumbnail_count: thumbnails.length,
                selected_thumbnail: selectedRoot?.querySelector('.input-media-thumbnail[aria-current="true"]')?.getAttribute('aria-label') ?? null,
                metadata_contains_second_attachment: selectedRoot?.querySelector('.input-media-metadata')?.textContent?.includes('media-node-000002') ?? false,
                source_contains_data_url: source.includes('data:image'),
                source_contains_base64_payload: source.includes('iVBORw0KGgo')
              }));
              selectedRoot?.querySelector('#diagnostics-tab')?.click();
              window.setTimeout(() => {
                const diagnosticsRoot = overlayRoot();
                console.warn('LENS_DIAGNOSTICS_RESULT=' + JSON.stringify({
                  displayed: Boolean(diagnosticsRoot?.querySelector('.diagnostics-header')),
                  selected: diagnosticsRoot?.querySelector('#diagnostics-tab')?.getAttribute('aria-selected') === 'true',
                  metric_count: diagnosticsRoot?.querySelectorAll('.metrics > div').length ?? 0,
                  quality: diagnosticsRoot?.querySelector('.quality')?.textContent?.trim() ?? null
                }));
                diagnosticsRoot?.querySelector('#interpretation-tab')?.click();
              }, 250);
              }, 500);
            }, 1000);
          })();
        "#;
            if let Err(error) = window.eval(script) {
                eprintln!("Unable to inspect rich-output validation DOM: {error}");
            }
        } else {
            eprintln!("Unable to inspect Source input-media validation window");
        }
        tokio::time::sleep(std::time::Duration::from_secs(19)).await;
        if let Err(error) = app_state::publish_lens_state(&validation_app, completed_state) {
            eprintln!("Unable to complete rich-output validation state: {error}");
        }
    });
    println!("LENS_RICH_OUTPUT_RESULT=displayed");
    Ok(())
}

#[cfg(debug_assertions)]
fn rich_output_validation_blocks() -> Result<Vec<model::LensOutputBlock>, String> {
    let json = match std::env::var_os("LENS_VALIDATE_ACP_UPDATES") {
        Some(path) => std::fs::read_to_string(path).map_err(|error| error.to_string())?,
        None => include_str!("../../tests/fixtures/acp-generated-image.json").into(),
    };
    #[derive(serde::Deserialize)]
    struct Notification {
        method: String,
        params: agent_client_protocol::schema::v1::SessionNotification,
    }
    let notifications: Vec<Notification> =
        serde_json::from_str(&json).map_err(|error| error.to_string())?;
    let mut candidate = agent_output::AgentOutputCandidate::default();
    for notification in notifications {
        if notification.method != "session/update" {
            return Err(
                "Rich output validation accepts only ACP session/update notifications".into(),
            );
        }
        candidate
            .record_update(notification.params.update, "read-only")
            .map_err(|error| error.to_string())?;
    }
    if !candidate.has_output() {
        return Err("Rich output validation produced no displayable output".into());
    }
    Ok(candidate.blocks())
}

#[cfg(all(test, debug_assertions))]
mod rich_output_validation_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    #[ignore = "requires LENS_VALIDATE_ACP_UPDATES with an explicit local notification replay"]
    fn replay_local_acp_images() {
        assert!(
            std::env::var_os("LENS_VALIDATE_ACP_UPDATES").is_some(),
            "Set LENS_VALIDATE_ACP_UPDATES to a JSON array of ACP session/update notifications"
        );
        let blocks = rich_output_validation_blocks().expect("normalize the local ACP replay");
        let mut image_count = 0;
        for block in blocks {
            if let model::LensOutputBlock::Image { data, .. } = block {
                let bytes = BASE64_STANDARD.decode(data).expect("decode image base64");
                let image = tauri::image::Image::from_bytes(&bytes).expect("decode image pixels");
                assert!(image.width() > 0 && image.height() > 0);
                image_count += 1;
                let digest: String = Sha256::digest(&bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                println!(
                    "ACP replay image {image_count}: {}x{}, {} bytes, SHA-256 {digest}",
                    image.width(),
                    image.height(),
                    bytes.len()
                );
            }
        }
        assert!(image_count > 0, "the local replay must contain an image");
    }
}
