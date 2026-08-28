use crate::{
    agent,
    app_state::{
        emit_app_snapshot, next_revision, publish_lens_state, update_lens_state, AgentRunKey,
        AppState,
    },
    lens::{
        target_id, LensContext, LensInput, LensMediaCapture, LensMediaOmission,
        LensMediaOmissionReason, LensMediaPayload, LensMediaPlan, LensMediaRequest, LensMediaScope,
        LensTargetCapture, LensTargetSet, MAX_AX_RESOURCE_REFERENCES, MAX_AX_RESOURCE_URI_BYTES,
        MAX_AX_TOTAL_RESOURCE_URI_BYTES, MAX_LENS_CONTEXT_NODES, MAX_LENS_CONTEXT_TEXT_BYTES,
        MAX_LENS_MEDIA_ATTACHMENTS, MAX_LENS_MEDIA_ATTACHMENT_BYTES, MAX_LENS_MEDIA_LONG_EDGE,
        MAX_LENS_MEDIA_PIXELS, MAX_LENS_MEDIA_TOTAL_BYTES, MAX_LENS_SOURCE_NODES,
        MAX_LENS_SOURCE_TEXT_BYTES, MAX_LENS_TARGETS,
    },
    model::{
        AgentKind, AgentSelectionState, AppConfig, AppSnapshot, LensStage, LensState,
        LensTargetSelection, LensTargetSelectionItem, LensTargetSelectionStage, SelectedWindow,
        BUILT_IN_RESPONSE_PROMPT,
    },
    platform::{self, ExtractionLimits, ImageCaptureLimits},
    ui,
};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

const OPERATION_SUPERSEDED: &str = "Lens operation was superseded by a newer selection";
const MAX_RESPONSE_PROMPT_CHARS: usize = 16_384;
const MAX_SELECTION_PREVIEW_LONG_EDGE: u32 = 480;
const MAX_SELECTION_PREVIEW_PIXELS: u32 = 230_400;
const MAX_SELECTION_PREVIEW_BYTES: u32 = 1024 * 1024;

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    state.snapshot()
}

pub async fn select_agent(
    app: AppHandle,
    candidate: AgentKind,
) -> Result<AgentSelectionState, String> {
    agent::select_agent(app, candidate).await
}

#[tauri::command]
pub async fn set_agent(app: AppHandle, agent: AgentKind) -> Result<AgentSelectionState, String> {
    select_agent(app, agent).await
}

#[tauri::command]
pub fn set_working_directory(path: String, app: AppHandle) -> Result<AppConfig, String> {
    update_working_directory(&app, PathBuf::from(path))
}

#[tauri::command]
pub fn set_response_prompt(response_prompt: String, app: AppHandle) -> Result<AppConfig, String> {
    let response_prompt = normalize_response_prompt(response_prompt)?;
    update_config(&app, |config| config.response_prompt = response_prompt)
}

fn normalize_response_prompt(response_prompt: String) -> Result<String, String> {
    let response_prompt = response_prompt.replace("\r\n", "\n").replace('\r', "\n");
    if response_prompt.trim().is_empty() {
        return Err("response prompt must not be empty".into());
    }
    if response_prompt.chars().count() > MAX_RESPONSE_PROMPT_CHARS {
        return Err(format!(
            "response prompt must not exceed {MAX_RESPONSE_PROMPT_CHARS} characters"
        ));
    }
    Ok(response_prompt)
}

#[tauri::command]
pub fn reset_response_prompt(app: AppHandle) -> Result<AppConfig, String> {
    update_config(&app, |config| {
        config.response_prompt = BUILT_IN_RESPONSE_PROMPT.into();
    })
}

pub fn update_working_directory(app: &AppHandle, directory: PathBuf) -> Result<AppConfig, String> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Err("working directory must be an existing absolute directory".into());
    }
    update_config(app, |config| config.working_directory = directory)
}

fn update_config(
    app: &AppHandle,
    update: impl FnOnce(&mut AppConfig),
) -> Result<AppConfig, String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned".to_string())?;
        let mut next = snapshot.config.clone();
        update(&mut next);
        let revision = next_revision(&snapshot)?;
        state.store.save(&next).map_err(|error| error.to_string())?;
        snapshot.config = next;
        snapshot.revision = revision;
        snapshot.clone()
    };
    let config = snapshot.config.clone();
    emit_app_snapshot(app, snapshot, true)?;
    Ok(config)
}

#[tauri::command]
pub fn accessibility_permission() -> bool {
    platform::accessibility_is_trusted()
}

#[tauri::command]
pub fn request_accessibility_permission() -> bool {
    platform::request_accessibility_trust()
}

async fn select_single_window(app: &AppHandle) -> Result<Option<SelectedWindow>, String> {
    let state = app.state::<AppState>();
    let _picker_lease = state.picker_control.try_begin()?;
    let reply = platform::present_window_picker()
        .await
        .map_err(|error| error.to_string())?;
    let Some(mut windows) = reply.into_selected()? else {
        return Ok(None);
    };
    if windows.len() != 1 {
        return Err(format!(
            "the single-window picker returned {} windows",
            windows.len()
        ));
    }
    Ok(windows.pop())
}

fn validation_window_count() -> usize {
    if std::env::var_os("LENS_VALIDATE_A11Y").is_none() {
        return 1;
    }
    std::env::var("LENS_VALIDATE_WINDOW_COUNT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| (1..=MAX_LENS_TARGETS).contains(count))
        .unwrap_or(1)
}

pub async fn select_and_extract(app: AppHandle) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let _ = state.agent_control.cancel_active()?;
    let operation_id = Uuid::new_v4();
    state.lens_media.begin(operation_id)?;
    publish_lens_state(
        &app,
        LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Selecting,
            ..LensState::default()
        },
    )?;

    let requested_windows = validation_window_count();
    let mut windows = Vec::with_capacity(requested_windows);
    for _ in 0..requested_windows {
        match select_single_window(&app).await {
            Ok(Some(window)) => windows.push(window),
            Ok(None) => {
                let cancelled = LensState {
                    operation_id: Some(operation_id),
                    stage: LensStage::Cancelled,
                    ..LensState::default()
                };
                if replace_operation_state(&app, operation_id, cancelled.clone())? {
                    return Ok(cancelled);
                }
                return Err(OPERATION_SUPERSEDED.into());
            }
            Err(message) => {
                replace_operation_state(
                    &app,
                    operation_id,
                    failed_state(operation_id, None, message.clone()),
                )?;
                return Err(message);
            }
        }
    }
    let target_set = match LensTargetSet::try_new(operation_id, windows) {
        Ok(target_set) => target_set,
        Err(error) => {
            let message = error.to_string();
            replace_operation_state(
                &app,
                operation_id,
                failed_state(operation_id, None, message.clone()),
            )?;
            return Err(message);
        }
    };

    let extracting = LensState {
        operation_id: Some(operation_id),
        stage: LensStage::Extracting,
        target_set: Some(target_set.clone()),
        ..LensState::default()
    };
    if !replace_operation_state(&app, operation_id, extracting)? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    extract_target_set_for_operation(app, operation_id, target_set).await
}

pub async fn extract_target(app: AppHandle, target: SelectedWindow) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let _ = state.agent_control.cancel_active()?;
    let operation_id = Uuid::new_v4();
    let target_set =
        LensTargetSet::try_new(operation_id, vec![target]).map_err(|error| error.to_string())?;
    state.lens_media.begin(operation_id)?;
    publish_lens_state(
        &app,
        LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Extracting,
            target_set: Some(target_set.clone()),
            ..LensState::default()
        },
    )?;
    extract_target_set_for_operation(app, operation_id, target_set).await
}

#[derive(Debug, Clone, Copy)]
struct SourceExtractionBudget {
    max_nodes: usize,
    max_text_bytes: usize,
    limits: ExtractionLimits,
}

fn source_extraction_budget(
    remaining_nodes: usize,
    remaining_text_bytes: usize,
    remaining_resource_refs: usize,
    remaining_resource_uri_bytes: usize,
) -> Option<SourceExtractionBudget> {
    let max_nodes = remaining_nodes.min(MAX_LENS_SOURCE_NODES);
    if max_nodes == 0 {
        return None;
    }
    let max_text_bytes = remaining_text_bytes.min(MAX_LENS_SOURCE_TEXT_BYTES);
    Some(SourceExtractionBudget {
        max_nodes,
        max_text_bytes,
        limits: ExtractionLimits {
            max_nodes: u32::try_from(max_nodes).expect("versioned node budget fits u32"),
            max_text_bytes: u32::try_from(max_text_bytes).expect("versioned text budget fits u32"),
            max_resource_refs: u32::try_from(remaining_resource_refs)
                .expect("versioned resource-reference budget fits u32"),
            max_resource_uri_bytes: u32::try_from(MAX_AX_RESOURCE_URI_BYTES)
                .expect("versioned resource-URI limit fits u32"),
            max_total_resource_uri_bytes: u32::try_from(remaining_resource_uri_bytes)
                .expect("versioned resource-URI group budget fits u32"),
        },
    })
}

async fn extract_target_set_for_operation(
    app: AppHandle,
    operation_id: Uuid,
    target_set: LensTargetSet,
) -> Result<LensState, String> {
    let mut remaining_nodes = MAX_LENS_CONTEXT_NODES;
    let mut remaining_text_bytes = MAX_LENS_CONTEXT_TEXT_BYTES;
    let mut remaining_resource_refs = MAX_AX_RESOURCE_REFERENCES;
    let mut remaining_resource_uri_bytes = MAX_AX_TOTAL_RESOURCE_URI_BYTES;
    let mut remaining_media_attachments = MAX_LENS_MEDIA_ATTACHMENTS;
    let mut remaining_media_bytes = MAX_LENS_MEDIA_TOTAL_BYTES;
    let mut captures = Vec::with_capacity(target_set.targets.len());
    let mut payloads = Vec::new();

    for target in &target_set.targets {
        let extraction_budget = source_extraction_budget(
            remaining_nodes,
            remaining_text_bytes,
            remaining_resource_refs,
            remaining_resource_uri_bytes,
        );
        let accessibility = if let Some(extraction_budget) = extraction_budget {
            let max_nodes = extraction_budget.max_nodes;
            let max_text_bytes = extraction_budget.max_text_bytes;
            let limits = extraction_budget.limits;
            let extraction_target = target.window.clone();
            let mut extraction = match tauri::async_runtime::spawn_blocking(move || {
                platform::extract_window(&extraction_target, limits)
            })
            .await
            {
                Ok(Ok(extraction)) => extraction,
                Ok(Err(error)) => LensContext::unavailable_accessibility(error.to_string()),
                Err(error) => LensContext::unavailable_accessibility(error.to_string()),
            };
            if extraction.metrics.truncated_nodes && max_nodes < MAX_LENS_SOURCE_NODES {
                extraction.diagnostics.push(format!(
                    "{} reached the version 1 group traversal-node budget",
                    target.id
                ));
            }
            if extraction.metrics.truncated_text && max_text_bytes < MAX_LENS_SOURCE_TEXT_BYTES {
                extraction.diagnostics.push(format!(
                    "{} reached the version 1 group text-byte budget",
                    target.id
                ));
            }
            remaining_nodes = remaining_nodes.saturating_sub(extraction.metrics.visited_nodes);
            remaining_text_bytes =
                remaining_text_bytes.saturating_sub(extraction.metrics.text_bytes);
            remaining_resource_refs =
                remaining_resource_refs.saturating_sub(extraction.metrics.resource_ref_count);
            remaining_resource_uri_bytes =
                remaining_resource_uri_bytes.saturating_sub(extraction.metrics.resource_uri_bytes);
            extraction
        } else {
            LensContext::unavailable_accessibility(format!(
                "{} was not extracted because the version 1 group traversal-node budget was exhausted",
                target.id
            ))
        };

        let plan = LensMediaPlan::from_accessibility(
            &target.window,
            &accessibility,
            remaining_media_attachments,
        );
        let mut media = capture_target_media(
            operation_id,
            target.id.clone(),
            target.window.clone(),
            plan,
            remaining_media_bytes,
        )
        .await;
        remaining_media_attachments =
            remaining_media_attachments.saturating_sub(media.attachments.len());
        remaining_media_bytes = remaining_media_bytes.saturating_sub(
            media
                .attachments
                .iter()
                .map(|attachment| attachment.encoded_bytes)
                .sum(),
        );
        payloads.append(&mut media.payloads);
        captures.push(LensTargetCapture {
            accessibility,
            media,
        });
    }

    let context = match LensContext::from_captures(operation_id, &target_set, captures) {
        Ok(context) => context,
        Err(error) => {
            let message = error.to_string();
            replace_operation_state(
                &app,
                operation_id,
                failed_state(operation_id, Some(target_set), message.clone()),
            )?;
            return Err(message);
        }
    };
    let input = LensInput::from_context(&context);
    let error = input.is_none().then(|| {
        if context.diagnostics.is_empty() {
            "Selected-window extraction did not yield usable structured content or media.".into()
        } else {
            context.diagnostics.join("; ")
        }
    });
    let next = LensState {
        operation_id: Some(operation_id),
        stage: if input.is_some() {
            LensStage::Ready
        } else {
            LensStage::Failed
        },
        selection: None,
        target_set: Some(target_set.clone()),
        context: Some(context),
        input,
        output_blocks: Vec::new(),
        agent: None,
        error,
    };
    if !app
        .state::<AppState>()
        .lens_media
        .replace(operation_id, payloads)?
    {
        return Err(OPERATION_SUPERSEDED.into());
    }
    if !replace_operation_state(&app, operation_id, next.clone())? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    if let Err(error) = ui::show_lens_window(&app, &target_set) {
        let message = format!("unable to show Lens window: {error}");
        if !update_lens_state(&app, operation_id, |lens| {
            lens.stage = LensStage::Failed;
            lens.error = Some(message.clone());
        })? {
            return Err(OPERATION_SUPERSEDED.into());
        }
        return Err(message);
    }
    Ok(next)
}

async fn capture_target_media(
    operation_id: Uuid,
    target_id: String,
    target: SelectedWindow,
    plan: LensMediaPlan,
    remaining_total_bytes: usize,
) -> LensMediaCapture {
    if plan.requests.is_empty() {
        return LensMediaCapture {
            omissions: plan.omissions,
            ..LensMediaCapture::default()
        };
    }
    if remaining_total_bytes == 0 {
        return byte_budget_media_capture(target_id, plan);
    }

    let capture_plan = plan.clone();
    let limits = ImageCaptureLimits {
        max_long_edge: u32::try_from(MAX_LENS_MEDIA_LONG_EDGE)
            .expect("versioned media long-edge limit fits u32"),
        max_pixels: u32::try_from(MAX_LENS_MEDIA_PIXELS)
            .expect("versioned media pixel limit fits u32"),
        max_attachment_bytes: u32::try_from(MAX_LENS_MEDIA_ATTACHMENT_BYTES)
            .expect("versioned media attachment-byte limit fits u32"),
        max_total_bytes: u32::try_from(remaining_total_bytes)
            .expect("versioned media total-byte budget fits u32"),
    };
    match tauri::async_runtime::spawn_blocking(move || {
        platform::capture_window_media(&target, operation_id, capture_plan, limits)
    })
    .await
    {
        Ok(Ok(capture)) => capture,
        Ok(Err(error)) => failed_media_capture(target_id, plan, error.to_string()),
        Err(error) => failed_media_capture(target_id, plan, error.to_string()),
    }
}

fn byte_budget_media_capture(target_id: String, plan: LensMediaPlan) -> LensMediaCapture {
    let mut omissions = plan.omissions;
    omissions.extend(plan.requests.into_iter().map(|request| LensMediaOmission {
        target_id: target_id.clone(),
        attachment_id: Some(request.id),
        source_node_id: request.source_node_id,
        reason: LensMediaOmissionReason::ByteBudget,
        omitted_count: 1,
        first_order: None,
        last_order: None,
        detail:
            "The versioned 16 MiB media group byte budget was exhausted before this target.".into(),
    }));
    LensMediaCapture {
        omissions,
        ..LensMediaCapture::default()
    }
}

fn failed_media_capture(
    target_id: String,
    plan: LensMediaPlan,
    message: String,
) -> LensMediaCapture {
    let mut omissions = plan.omissions;
    omissions.extend(plan.requests.into_iter().map(|request| LensMediaOmission {
        target_id: target_id.clone(),
        attachment_id: Some(request.id),
        source_node_id: request.source_node_id,
        reason: LensMediaOmissionReason::CaptureFailed,
        omitted_count: 1,
        first_order: None,
        last_order: None,
        detail: message.clone(),
    }));
    LensMediaCapture {
        omissions,
        diagnostics: vec![format!("ScreenCaptureKit media capture failed: {message}")],
        ..LensMediaCapture::default()
    }
}

async fn build_target_selection_item(
    operation_id: Uuid,
    window: SelectedWindow,
) -> (LensTargetSelectionItem, Option<LensMediaPayload>) {
    let id = target_id(&window);
    let attachment_id = format!("selection-preview-window-{}", window.window_id);
    let plan = LensMediaPlan {
        requests: vec![LensMediaRequest {
            id: attachment_id,
            scope: LensMediaScope::WindowFallback,
            source_node_id: None,
            bounds: None,
        }],
        omissions: Vec::new(),
    };
    let capture_window = window.clone();
    let capture = tauri::async_runtime::spawn_blocking(move || {
        platform::capture_window_media(
            &capture_window,
            operation_id,
            plan,
            ImageCaptureLimits {
                max_long_edge: MAX_SELECTION_PREVIEW_LONG_EDGE,
                max_pixels: MAX_SELECTION_PREVIEW_PIXELS,
                max_attachment_bytes: MAX_SELECTION_PREVIEW_BYTES,
                max_total_bytes: MAX_SELECTION_PREVIEW_BYTES,
            },
        )
    })
    .await;

    let (preview_uri, preview_error, payload) = match capture {
        Ok(Ok(mut capture)) if capture.payloads.len() == 1 => {
            let mut payload = capture.payloads.remove(0);
            let uri = format!(
                "lens://selection/{operation_id}/window/{}",
                window.window_id
            );
            payload.uri = uri.clone();
            (Some(uri), None, Some(payload))
        }
        Ok(Ok(capture)) => {
            let detail = capture
                .diagnostics
                .first()
                .cloned()
                .or_else(|| {
                    capture
                        .omissions
                        .first()
                        .map(|omission| omission.detail.clone())
                })
                .unwrap_or_else(|| {
                    format!(
                        "preview capture returned {} payloads instead of one",
                        capture.payloads.len()
                    )
                });
            (None, Some(detail), None)
        }
        Ok(Err(error)) => (None, Some(error.to_string()), None),
        Err(error) => (None, Some(error.to_string()), None),
    };

    (
        LensTargetSelectionItem {
            id,
            window,
            preview_uri,
            preview_error,
        },
        payload,
    )
}

fn target_selection_for_operation(
    app: &AppHandle,
    operation_id: Uuid,
) -> Result<LensTargetSelection, String> {
    let lens = app.state::<AppState>().lens()?;
    if lens.operation_id != Some(operation_id) || lens.stage != LensStage::Selecting {
        return Err(OPERATION_SUPERSEDED.into());
    }
    let selection = lens
        .selection
        .ok_or_else(|| "Lens target selection state is unavailable".to_string())?;
    if selection.selection_id != operation_id
        || selection.maximum_targets != MAX_LENS_TARGETS
        || selection.items.len() > selection.maximum_targets
        || selection.anchor.is_some() != !selection.items.is_empty()
    {
        return Err("Lens target selection state violates its versioned invariants".into());
    }
    Ok(selection)
}

async fn publish_target_selection(
    app: &AppHandle,
    operation_id: Uuid,
    selection: LensTargetSelection,
) -> Result<LensState, String> {
    let next = LensState {
        operation_id: Some(operation_id),
        stage: LensStage::Selecting,
        selection: Some(selection.clone()),
        ..LensState::default()
    };
    if !replace_operation_state(app, operation_id, next.clone())? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    if selection.items.is_empty() {
        return Ok(next);
    }
    if let Err(error) = ui::show_target_selection_window(app, &selection).await {
        let message = error.to_string();
        let _ = ui::destroy_target_selection_window(app);
        replace_operation_state(
            app,
            operation_id,
            failed_state(operation_id, None, message.clone()),
        )?;
        return Err(message);
    }
    Ok(next)
}

fn store_target_selection_payload(
    app: &AppHandle,
    operation_id: Uuid,
    payload: Option<LensMediaPayload>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut payloads = state.lens_media.payloads(operation_id)?;
    if let Some(payload) = payload {
        payloads.push(payload);
    }
    if !state.lens_media.replace(operation_id, payloads)? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    Ok(())
}

#[tauri::command]
pub async fn select_lens_target(app: AppHandle) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let agent_selection = state.agent_selection()?;
    if !agent_selection.can_select_lens_target() {
        return Err("select and authenticate an AI Agent before selecting a Lens Target".into());
    }
    let _ = state.agent_control.cancel_active()?;
    let operation_id = Uuid::new_v4();
    state.lens_media.begin(operation_id)?;
    let initial_selection = LensTargetSelection {
        selection_id: operation_id,
        stage: LensTargetSelectionStage::Picking,
        maximum_targets: MAX_LENS_TARGETS,
        anchor: None,
        items: Vec::new(),
        notice: None,
    };
    publish_lens_state(
        &app,
        LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Selecting,
            selection: Some(initial_selection),
            ..LensState::default()
        },
    )?;

    let window = match select_single_window(&app).await {
        Ok(Some(window)) => window,
        Ok(None) => {
            let cancelled = LensState {
                operation_id: Some(operation_id),
                stage: LensStage::Cancelled,
                ..LensState::default()
            };
            if replace_operation_state(&app, operation_id, cancelled.clone())? {
                return Ok(cancelled);
            }
            return Err(OPERATION_SUPERSEDED.into());
        }
        Err(message) => {
            replace_operation_state(
                &app,
                operation_id,
                failed_state(operation_id, None, message.clone()),
            )?;
            return Err(message);
        }
    };
    let anchor = window.frame;
    let (item, payload) = build_target_selection_item(operation_id, window).await;
    store_target_selection_payload(&app, operation_id, payload)?;
    publish_target_selection(
        &app,
        operation_id,
        LensTargetSelection {
            selection_id: operation_id,
            stage: LensTargetSelectionStage::Reviewing,
            maximum_targets: MAX_LENS_TARGETS,
            anchor: Some(anchor),
            items: vec![item],
            notice: None,
        },
    )
    .await
}

#[tauri::command]
pub async fn add_lens_target(app: AppHandle, operation_id: Uuid) -> Result<LensState, String> {
    let mut selection = target_selection_for_operation(&app, operation_id)?;
    if selection.stage != LensTargetSelectionStage::Reviewing {
        return Err("Lens target selection is already picking a window".into());
    }
    if selection.items.len() >= selection.maximum_targets {
        return Err(format!(
            "Lens target selection is limited to {} windows",
            selection.maximum_targets
        ));
    }
    selection.stage = LensTargetSelectionStage::Picking;
    selection.notice = None;
    publish_target_selection(&app, operation_id, selection.clone()).await?;

    let selected = select_single_window(&app).await;
    selection = target_selection_for_operation(&app, operation_id)?;
    match selected {
        Ok(None) => {
            selection.stage = LensTargetSelectionStage::Reviewing;
            publish_target_selection(&app, operation_id, selection).await
        }
        Err(error) => {
            selection.stage = LensTargetSelectionStage::Reviewing;
            selection.notice = Some(format!("Unable to add a window: {error}"));
            publish_target_selection(&app, operation_id, selection).await?;
            Err(error)
        }
        Ok(Some(window)) => {
            if selection
                .items
                .iter()
                .any(|item| item.window.window_id == window.window_id)
            {
                selection.stage = LensTargetSelectionStage::Reviewing;
                selection.notice = Some("That window is already selected.".into());
                return publish_target_selection(&app, operation_id, selection).await;
            }
            let (item, payload) = build_target_selection_item(operation_id, window).await;
            store_target_selection_payload(&app, operation_id, payload)?;
            selection.items.push(item);
            selection.stage = LensTargetSelectionStage::Reviewing;
            selection.notice = None;
            publish_target_selection(&app, operation_id, selection).await
        }
    }
}

#[tauri::command]
pub async fn remove_lens_target(
    app: AppHandle,
    operation_id: Uuid,
    target_id: String,
) -> Result<LensState, String> {
    let mut selection = target_selection_for_operation(&app, operation_id)?;
    if selection.stage != LensTargetSelectionStage::Reviewing {
        return Err("Lens target selection cannot be edited while the picker is active".into());
    }
    let index = selection
        .items
        .iter()
        .position(|item| item.id == target_id)
        .ok_or_else(|| "Lens target selection item is unavailable".to_string())?;
    let removed = selection.items.remove(index);
    let state = app.state::<AppState>();
    let payloads = state
        .lens_media
        .payloads(operation_id)?
        .into_iter()
        .filter(|payload| removed.preview_uri.as_deref() != Some(payload.uri.as_str()))
        .collect();
    if !state.lens_media.replace(operation_id, payloads)? {
        return Err(OPERATION_SUPERSEDED.into());
    }

    if selection.items.is_empty() {
        ui::dismiss_target_selection_window(&app)
            .await
            .map_err(|error| error.to_string())?;
        let cancelled = LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Cancelled,
            ..LensState::default()
        };
        if replace_operation_state(&app, operation_id, cancelled.clone())? {
            return Ok(cancelled);
        }
        return Err(OPERATION_SUPERSEDED.into());
    }
    selection.notice = None;
    publish_target_selection(&app, operation_id, selection).await
}

#[tauri::command]
pub async fn confirm_lens_targets(app: AppHandle, operation_id: Uuid) -> Result<LensState, String> {
    let selection = target_selection_for_operation(&app, operation_id)?;
    if selection.stage != LensTargetSelectionStage::Reviewing {
        return Err("Lens target selection cannot be confirmed while the picker is active".into());
    }
    let windows = selection
        .items
        .into_iter()
        .map(|item| item.window)
        .collect();
    let target_set =
        LensTargetSet::try_new(operation_id, windows).map_err(|error| error.to_string())?;
    ui::dismiss_target_selection_window(&app)
        .await
        .map_err(|error| error.to_string())?;
    let extracting = LensState {
        operation_id: Some(operation_id),
        stage: LensStage::Extracting,
        target_set: Some(target_set.clone()),
        ..LensState::default()
    };
    if !replace_operation_state(&app, operation_id, extracting)? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    let ready = extract_target_set_for_operation(app.clone(), operation_id, target_set).await?;
    if ready.stage != LensStage::Ready {
        return Ok(ready);
    }
    agent::transform_current(app, operation_id).await
}

#[tauri::command]
pub async fn transform_lens(app: AppHandle, operation_id: Uuid) -> Result<LensState, String> {
    agent::transform_current(app, operation_id).await
}

#[tauri::command]
pub async fn authenticate_agent(
    app: AppHandle,
    operation_id: Uuid,
    method_id: String,
) -> Result<LensState, String> {
    agent::authenticate_current(app, operation_id, method_id).await
}

#[tauri::command]
pub async fn authenticate_agent_selection(
    app: AppHandle,
    method_id: String,
) -> Result<AgentSelectionState, String> {
    agent::authenticate_selection(app, method_id).await
}

#[tauri::command]
pub async fn reauthenticate_agent_selection(app: AppHandle) -> Result<AgentSelectionState, String> {
    agent::reauthenticate_selection(app).await
}

#[tauri::command]
pub async fn sign_out_agent_selection(app: AppHandle) -> Result<AgentSelectionState, String> {
    agent::sign_out_selection(app).await
}

#[tauri::command]
pub fn cancel_agent(app: AppHandle, operation_id: Uuid, run_id: Uuid) -> Result<LensState, String> {
    agent::cancel_current(
        &app,
        AgentRunKey {
            operation_id,
            run_id,
        },
    )
}

#[tauri::command]
pub fn show_settings(app: AppHandle) -> Result<(), String> {
    ui::show_settings(&app).map_err(|error| error.to_string())
}

fn failed_state(operation_id: Uuid, target_set: Option<LensTargetSet>, error: String) -> LensState {
    LensState {
        operation_id: Some(operation_id),
        stage: LensStage::Failed,
        target_set,
        error: Some(error),
        ..LensState::default()
    }
}

fn replace_operation_state(
    app: &AppHandle,
    operation_id: Uuid,
    next: LensState,
) -> Result<bool, String> {
    update_lens_state(app, operation_id, |state| *state = next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_prompt_is_non_empty_and_has_deterministic_line_endings() {
        assert_eq!(
            normalize_response_prompt("First\r\nSecond\rThird".into()),
            Ok("First\nSecond\nThird".into())
        );
        assert_eq!(
            normalize_response_prompt(" \n\t".into()),
            Err("response prompt must not be empty".into())
        );
        assert!(normalize_response_prompt("x".repeat(MAX_RESPONSE_PROMPT_CHARS)).is_ok());
        assert_eq!(
            normalize_response_prompt("x".repeat(MAX_RESPONSE_PROMPT_CHARS + 1)),
            Err(format!(
                "response prompt must not exceed {MAX_RESPONSE_PROMPT_CHARS} characters"
            ))
        );
    }

    #[test]
    fn failed_operation_preserves_identity_and_target() {
        let operation_id = Uuid::new_v4();
        let target = SelectedWindow {
            window_id: 42,
            title: "Document".into(),
            application_name: "Browser".into(),
            bundle_id: "example.browser".into(),
            pid: 100,
            frame: crate::model::Bounds {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
            },
        };

        let target_set =
            LensTargetSet::try_new(operation_id, vec![target]).expect("valid target set");
        let state = failed_state(operation_id, Some(target_set.clone()), "failure".into());

        assert_eq!(state.operation_id, Some(operation_id));
        assert_eq!(state.stage, LensStage::Failed);
        assert_eq!(state.target_set, Some(target_set));
        assert_eq!(state.error.as_deref(), Some("failure"));
    }

    #[test]
    fn exhausted_diagnostic_text_budget_keeps_source_traversal_available() {
        let budget = source_extraction_budget(
            MAX_LENS_SOURCE_NODES,
            0,
            MAX_AX_RESOURCE_REFERENCES,
            MAX_AX_TOTAL_RESOURCE_URI_BYTES,
        )
        .expect("remaining nodes keep extraction available");

        assert_eq!(budget.max_nodes, MAX_LENS_SOURCE_NODES);
        assert_eq!(budget.max_text_bytes, 0);
        assert_eq!(budget.limits.max_text_bytes, 0);
        assert!(source_extraction_budget(
            0,
            MAX_LENS_SOURCE_TEXT_BYTES,
            MAX_AX_RESOURCE_REFERENCES,
            MAX_AX_TOTAL_RESOURCE_URI_BYTES,
        )
        .is_none());
    }
}
