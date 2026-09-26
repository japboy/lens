//! Opt-in native WebView/IPC acceptance probe; never compiled into release builds.
use crate::{app_state, lens, live_sync::CanonicalProjection, model, ui};
use std::time::Duration;
use tauri::{Listener, Manager};
use uuid::Uuid;

fn append(state: &mut model::LensState, ordinal: u64) -> Result<(), String> {
    let operation_id = state.operation_id.ok_or("Missing validation operation")?;
    let projection = CanonicalProjection::from_serializable(&ordinal)
        .map_err(|error| error.to_string())?
        .projection_ref(std::num::NonZeroU64::new(ordinal).ok_or("Invalid ordinal")?);
    let html = format!(
        "<!doctype html><html><body style=\"font:24px system-ui;padding:24px\"><h1>Retained visual {ordinal}</h1><p>Native response history validation</p></body></html>"
    );
    let text = format!(
        "## Response {ordinal}\n\nNative cumulative output **{ordinal}**.\n\n{}\n\nInline math: \\(x^2 + y^2 = z^2\\).",
        "A retained paragraph remains available after subsequent publication.\n\n".repeat(24)
    );
    let mut blocks = vec![
        model::LensOutputBlock::Html {
            message_id: None,
            resource_id: "retained-validation-html".into(),
            mime_type: "text/html".into(),
            uri: "lens-output://html/retained-validation-html".into(),
            byte_length: html.len(),
            text: html,
        },
        model::LensOutputBlock::Markdown {
            message_id: None,
            text,
        },
    ];
    // Reuse an existing synthetic Agent image; no live capture or Agent is involved.
    if let Some(image) = state
        .output_blocks
        .iter()
        .find(|block| matches!(block, model::LensOutputBlock::Image { .. }))
    {
        blocks.push(image.clone());
    }
    let representation = model::LensRepresentation {
        prompt_execution_revision: state.prompt_execution_revision,
        representation_id: Uuid::new_v4(),
        context_id: operation_id,
        context_revision: ordinal,
        projection: projection.clone(),
        run_id: Uuid::new_v4(),
        output_blocks: blocks.into(),
    };
    state
        .response_history
        .append(representation.clone(), Some("native-validation".into()))?;
    state.representation = Some(representation);
    state.projection = Some(projection);
    state.stage = model::LensStage::Completed;
    Ok(())
}

pub(crate) fn run<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    mut state: model::LensState,
    target_set: lens::LensTargetSet,
) -> Result<(), String> {
    append(&mut state, 1)?;
    let operation = state.operation_id.ok_or("Missing validation operation")?;
    let first = state
        .representation
        .as_ref()
        .ok_or("Missing first response")?
        .representation_id;
    let (ready_tx, mut ready_rx) = tokio::sync::watch::channel(false);
    let ready_listener = app.listen("lens-response-history-ready", move |_| {
        let _ = ready_tx.send(true);
    });
    let result_app = app.clone();
    let result_listener = app.listen("lens-response-history-result", move |event| {
        println!("LENS_RESPONSE_HISTORY_RESULT={}", event.payload());
        let passed = serde_json::from_str::<serde_json::Value>(event.payload())
            .ok()
            .and_then(|result| result["passed"].as_bool())
            .unwrap_or(false);
        result_app.exit(if passed { 0 } else { 1 });
    });
    app_state::publish_lens_state(&app, state.clone())?;
    ui::show_lens_window(&app, &target_set).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn(async move {
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let window = app
                .get_webview_window(ui::LENS_WINDOW_LABEL)
                .ok_or("Missing validation Overlay")?;
            let script = include_str!("response_history_validation.js")
                .replace("__OPERATION_ID__", &operation.to_string())
                .replace("__REPRESENTATION_ID__", &first.to_string());
            window.eval(script).map_err(|error| error.to_string())?;
            ready_rx
                .changed()
                .await
                .map_err(|error| error.to_string())?;
            for ordinal in 2..=3 {
                append(&mut state, ordinal)?;
                app_state::publish_lens_state(&app, state.clone())?;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            // The WebView reports its result through the scoped debug listener.
            tokio::time::sleep(Duration::from_secs(40)).await;
            Err::<(), String>("WebView did not report a validation result".into())
        })
        .await;
        app.unlisten(ready_listener);
        app.unlisten(result_listener);
        eprintln!(
            "LENS_RESPONSE_HISTORY_RESULT={{\"passed\":false,\"error\":{:?}}}",
            format!("{result:?}")
        );
        app.exit(1);
    });
    Ok(())
}

/// Restore synthetic provider replay through the real SessionView manifest/IPC path.
pub(crate) fn run_replay<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    target_set: lens::LensTargetSet,
) -> Result<(), String> {
    use crate::session_document::{DocumentBlock, DocumentEntry, MessageRole, SessionDocument};
    use agent_client_protocol::schema::v1::ToolCallStatus;
    let mut document = SessionDocument::default();
    for ordinal in 1..=3 {
        document.entries.push(DocumentEntry::Message {
            id: format!("user-{ordinal}"),
            role: MessageRole::User,
            blocks: vec![DocumentBlock::Markdown {
                text: format!("Replay prompt {ordinal}"),
            }],
        });
        if ordinal <= 2 {
            document.entries.push(DocumentEntry::Tool {
                id: format!("html-{ordinal}"),
                title: "Completed HTML publication".into(),
                status: ToolCallStatus::Completed,
                blocks: vec![],
                accepted_html: Some(format!(
                    "<!doctype html><html><body><h1>Replay visual {ordinal}</h1>{}</body></html>",
                    "<p>A long retained HTML paragraph.</p>".repeat(40)
                )),
            });
        }
        document.entries.push(DocumentEntry::Message {
            id: format!("answer-{ordinal}"),
            role: MessageRole::Assistant,
            blocks: vec![DocumentBlock::Markdown {
                text: format!(
                    "## Replay response {ordinal}\n\nReplay body #{ordinal}.\n\n{}",
                    "A retained historical paragraph.\n\n".repeat(12)
                ),
            }],
        });
    }
    // This failed trailing publication must not erase earlier output or enter Hero.
    document.entries.push(DocumentEntry::Tool {
        id: "failed-html".into(),
        title: "Failed publication".into(),
        status: ToolCallStatus::Failed,
        blocks: vec![DocumentBlock::Html {
            text: "<h1>Failed content must stay out of Hero</h1>".into(),
        }],
        accepted_html: None,
    });
    let state = app.state::<app_state::AppState>();
    state.session_view.install_validation_document(document)?;
    crate::session_view::emit(&app)?;
    ui::show_lens_window(&app, &target_set).map_err(|error| error.to_string())?;
    let result_app = app.clone();
    app.listen("lens-history-replay-result", move |event| {
        println!("LENS_HISTORY_REPLAY_RESULT={}", event.payload());
        let passed = serde_json::from_str::<serde_json::Value>(event.payload())
            .ok()
            .and_then(|result| result["passed"].as_bool())
            .unwrap_or(false);
        result_app.exit(if passed { 0 } else { 1 });
    });
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Some(window) = app.get_webview_window(ui::LENS_WINDOW_LABEL) {
            if let Err(error) = window.eval(include_str!("history_replay_validation.js")) {
                eprintln!("History replay validation could not start: {error}");
                app.exit(1);
                return;
            }
        }
        tokio::time::sleep(Duration::from_secs(45)).await;
        eprintln!("LENS_HISTORY_REPLAY_RESULT={{\"passed\":false,\"error\":\"WebView validation timed out\"}}");
        app.exit(1);
    });
    Ok(())
}
