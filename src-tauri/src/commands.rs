use crate::{
    agent,
    app_state::{
        emit_app_snapshot, next_revision, publish_lens_state, update_lens_state, AgentRunKey,
        AppState,
    },
    model::{
        AgentKind, AgentSelectionState, AppConfig, AppSnapshot, LensInput, LensStage, LensState,
        SelectedWindow, BUILT_IN_RESPONSE_PROMPT,
    },
    platform, ui,
};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

const OPERATION_SUPERSEDED: &str = "Lens operation was superseded by a newer selection";

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

pub async fn select_and_extract(app: AppHandle) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let picker_lease = state.picker_control.try_begin()?;
    let _ = state.agent_control.cancel_active()?;
    let operation_id = Uuid::new_v4();
    publish_lens_state(
        &app,
        LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Selecting,
            ..LensState::default()
        },
    )?;

    let picker_reply = match platform::present_window_picker().await {
        Ok(reply) => reply,
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
    let selected = match picker_reply.into_selected() {
        Ok(selected) => selected,
        Err(error) => {
            let failed = failed_state(operation_id, None, error.clone());
            replace_operation_state(&app, operation_id, failed)?;
            return Err(error);
        }
    };
    let Some(target) = selected else {
        let cancelled = LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Cancelled,
            ..LensState::default()
        };
        if replace_operation_state(&app, operation_id, cancelled.clone())? {
            return Ok(cancelled);
        }
        return Err(OPERATION_SUPERSEDED.into());
    };

    let extracting = LensState {
        operation_id: Some(operation_id),
        stage: LensStage::Extracting,
        target: Some(target.clone()),
        ..LensState::default()
    };
    if !replace_operation_state(&app, operation_id, extracting)? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    drop(picker_lease);

    extract_target_for_operation(app, operation_id, target).await
}

pub async fn extract_target(app: AppHandle, target: SelectedWindow) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let _ = state.agent_control.cancel_active()?;
    let operation_id = Uuid::new_v4();
    publish_lens_state(
        &app,
        LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Extracting,
            target: Some(target.clone()),
            ..LensState::default()
        },
    )?;
    extract_target_for_operation(app, operation_id, target).await
}

async fn extract_target_for_operation(
    app: AppHandle,
    operation_id: Uuid,
    target: SelectedWindow,
) -> Result<LensState, String> {
    let extraction_target = target.clone();
    let extraction = match tauri::async_runtime::spawn_blocking(move || {
        platform::extract_window(&extraction_target)
    })
    .await
    {
        Ok(Ok(extraction)) => extraction,
        Ok(Err(error)) => {
            let message = error.to_string();
            let failed = failed_state(operation_id, Some(target.clone()), message.clone());
            if !replace_operation_state(&app, operation_id, failed)? {
                return Err(OPERATION_SUPERSEDED.into());
            }
            ui::show_overlay(&app, &target, operation_id).map_err(|overlay_error| {
                format!("{message}; unable to show Lens overlay: {overlay_error}")
            })?;
            return Err(message);
        }
        Err(error) => {
            let message = error.to_string();
            let failed = failed_state(operation_id, Some(target.clone()), message.clone());
            if !replace_operation_state(&app, operation_id, failed)? {
                return Err(OPERATION_SUPERSEDED.into());
            }
            ui::show_overlay(&app, &target, operation_id).map_err(|overlay_error| {
                format!("{message}; unable to show Lens overlay: {overlay_error}")
            })?;
            return Err(message);
        }
    };
    let input = LensInput::from_extraction(&target, &extraction);
    let next = LensState {
        operation_id: Some(operation_id),
        stage: if input.is_some() {
            LensStage::Ready
        } else {
            LensStage::Failed
        },
        target: Some(target.clone()),
        extraction: Some(extraction.clone()),
        input,
        output_blocks: Vec::new(),
        agent: None,
        error: if extraction.quality == crate::model::ExtractionQuality::Unavailable {
            Some("Accessibility extraction did not yield usable text.".into())
        } else {
            None
        },
    };
    if !replace_operation_state(&app, operation_id, next.clone())? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    if let Err(error) = ui::show_overlay(&app, &target, operation_id) {
        let message = format!("unable to show Lens overlay: {error}");
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

#[tauri::command]
pub async fn select_lens_target(app: AppHandle) -> Result<LensState, String> {
    let agent_selection = app.state::<AppState>().agent_selection()?;
    if !agent_selection.can_select_lens_target() {
        return Err("select and authenticate an AI Agent before selecting a Lens Target".into());
    }
    let ready = select_and_extract(app.clone()).await?;
    if ready.stage == LensStage::Ready {
        agent::transform_current(
            app,
            ready
                .operation_id
                .ok_or_else(|| "ready Lens operation has no identity".to_string())?,
        )
        .await
    } else {
        Ok(ready)
    }
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

fn failed_state(operation_id: Uuid, target: Option<SelectedWindow>, error: String) -> LensState {
    LensState {
        operation_id: Some(operation_id),
        stage: LensStage::Failed,
        target,
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

        let state = failed_state(operation_id, Some(target.clone()), "failure".into());

        assert_eq!(state.operation_id, Some(operation_id));
        assert_eq!(state.stage, LensStage::Failed);
        assert_eq!(state.target, Some(target));
        assert_eq!(state.error.as_deref(), Some("failure"));
    }
}
