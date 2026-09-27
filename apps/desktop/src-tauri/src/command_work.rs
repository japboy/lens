//! Bounded IPC execution. Acquire capacity before scheduling blocking native work.
use crate::{app_state::AppState, commands, model::AppConfig};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

#[derive(Debug)]
pub(crate) struct WorkSlots {
    control: std::sync::Arc<tokio::sync::Semaphore>,
    settings: std::sync::Arc<tokio::sync::Semaphore>,
    content: std::sync::Arc<tokio::sync::Semaphore>,
}

impl Default for WorkSlots {
    fn default() -> Self {
        Self {
            control: std::sync::Arc::new(tokio::sync::Semaphore::new(16)),
            settings: std::sync::Arc::new(tokio::sync::Semaphore::new(16)),
            content: std::sync::Arc::new(tokio::sync::Semaphore::new(8)),
        }
    }
}

enum Lane {
    Control,
    Settings,
    Content,
}

async fn run<R: tauri::Runtime, T: Send + 'static>(
    app: AppHandle<R>,
    lane: Lane,
    work: impl FnOnce(AppHandle<R>) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let permit = {
        let state = app.state::<AppState>();
        let slots = &state.store.command_workers;
        let slots = match lane {
            Lane::Control => &slots.control,
            Lane::Settings => &slots.settings,
            Lane::Content => &slots.content,
        };
        slots.clone().try_acquire_owned()
            .map_err(|_| "Too many pending native commands in this category. Retry after an operation completes.".to_string())?
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        work(app)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub(crate) async fn blocking<R: tauri::Runtime, T: Send + 'static>(
    app: AppHandle<R>,
    work: impl FnOnce(AppHandle<R>) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    run(app, Lane::Control, work).await
}

pub(crate) async fn configuration<R: tauri::Runtime, T: Send + 'static>(
    app: AppHandle<R>,
    work: impl FnOnce(AppHandle<R>) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    run(app, Lane::Settings, work).await
}

async fn content<R: tauri::Runtime, T: Send + 'static>(
    app: AppHandle<R>,
    work: impl FnOnce(AppHandle<R>) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    run(app, Lane::Content, work).await
}

#[tauri::command]
pub async fn delete_external_agent<R: tauri::Runtime>(
    app: AppHandle<R>,
    id: Uuid,
) -> Result<AppConfig, String> {
    configuration(app, move |app| commands::delete_external_agent(app, id)).await
}

#[tauri::command]
pub async fn reset_external_agents<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<AppConfig, String> {
    configuration(app, move |app| commands::reset_external_agents(app)).await
}

#[tauri::command]
pub async fn set_working_directory<R: tauri::Runtime>(
    app: AppHandle<R>,
    path: String,
) -> Result<AppConfig, String> {
    configuration(app, move |app| commands::set_working_directory(path, app)).await
}

#[tauri::command]
pub async fn set_agent_prompt_template<R: tauri::Runtime>(
    app: AppHandle<R>,
    agent_prompt_template: crate::prompt_template::AgentPromptTemplate,
) -> Result<AppConfig, String> {
    configuration(app, move |app| {
        commands::set_agent_prompt_template(agent_prompt_template, app)
    })
    .await
}

#[tauri::command]
pub async fn reset_agent_prompt_template<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<AppConfig, String> {
    configuration(app, move |app| commands::reset_agent_prompt_template(app)).await
}

#[tauri::command]
pub async fn update_prompt_presets<R: tauri::Runtime>(
    app: AppHandle<R>,
    change: usecase::prompt_presets::PromptPresetMutation,
) -> Result<AppConfig, String> {
    configuration(app, move |app| commands::update_prompt_presets(change, app)).await
}

#[tauri::command]
pub async fn pause_lens<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<(), String> {
    blocking(app, move |app| {
        commands::pause_lens(app, operation_id).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn resume_lens<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<(), String> {
    blocking(app, move |app| {
        commands::resume_lens(app, operation_id).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn stop_lens<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<(), String> {
    blocking(app, move |app| {
        commands::stop_lens(app, operation_id).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn cancel_agent<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    run_id: Uuid,
) -> Result<(), String> {
    blocking(app, move |app| {
        commands::cancel_agent(app, operation_id, run_id).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn set_session_option<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    instance_id: Uuid,
    config_revision: u32,
    config_id: String,
    value: String,
) -> Result<(), String> {
    blocking(app, move |app| {
        commands::set_session_option(
            app,
            operation_id,
            instance_id,
            config_revision,
            config_id,
            value,
        )
    })
    .await
}

#[tauri::command]
pub async fn respond_agent_interaction<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    instance_id: Uuid,
    interaction_id: Uuid,
    response: crate::session_controls::InteractionResponse,
) -> Result<(), String> {
    blocking(app, move |app| {
        commands::respond_agent_interaction(
            app,
            operation_id,
            instance_id,
            interaction_id,
            response,
        )
    })
    .await
}

#[tauri::command]
pub async fn select_lens_target<R: tauri::Runtime>(app: AppHandle<R>) -> Result<(), String> {
    commands::select_lens_target(app).await.map(|_| ())
}

#[tauri::command]
pub async fn add_lens_target<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<(), String> {
    commands::add_lens_target(app, operation_id)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn remove_lens_target<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    target_id: String,
) -> Result<(), String> {
    commands::remove_lens_target(app, operation_id, target_id)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn confirm_lens_targets<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<(), String> {
    commands::confirm_lens_targets(app, operation_id)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn retry_lens_transform<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<(), String> {
    commands::retry_lens_transform(app, operation_id)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn authenticate_agent<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    method_id: String,
) -> Result<(), String> {
    commands::authenticate_agent(app, operation_id, method_id)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn get_window_snapshot<R: tauri::Runtime>(
    app: AppHandle<R>,
    webview: tauri::Webview<R>,
) -> Result<crate::publication::WindowSnapshot, String> {
    let label = webview.label().to_string();
    content(app, move |app| {
        crate::publication::window_snapshot(&app.state::<AppState>(), &label)
    })
    .await
}

#[tauri::command]
pub async fn get_lens_source<R: tauri::Runtime>(
    app: AppHandle<R>,
    webview: tauri::Webview<R>,
    source_ref: String,
) -> Result<crate::publication::LensSourceContent, String> {
    if webview.label() != "lens-overlay" {
        return Err("Lens content is only available to the Lens overlay".into());
    }
    content(app, move |app| {
        crate::publication::source(&app.state::<AppState>(), &source_ref)
    })
    .await
}

#[tauri::command]
pub async fn get_lens_output<R: tauri::Runtime>(
    app: AppHandle<R>,
    webview: tauri::Webview<R>,
    output_ref: String,
) -> Result<crate::publication::LensOutputContent, String> {
    if webview.label() != "lens-overlay" {
        return Err("Lens content is only available to the Lens overlay".into());
    }
    content(app, move |app| {
        crate::publication::output(&app.state::<AppState>(), &output_ref)
    })
    .await
}

#[tauri::command]
pub async fn get_lens_image<R: tauri::Runtime>(
    app: AppHandle<R>,
    webview: tauri::Webview<R>,
    image_ref: String,
) -> Result<crate::publication::LensImageContent, String> {
    if webview.label() != "lens-overlay" {
        return Err("Lens content is only available to the Lens overlay".into());
    }
    content(app, move |app| {
        crate::publication::image(&app.state::<AppState>(), &image_ref)
    })
    .await
}

#[tauri::command]
pub async fn get_response_block<R: tauri::Runtime>(
    app: AppHandle<R>,
    webview: tauri::Webview<R>,
    operation_id: Uuid,
    representation_id: Uuid,
    block_index: usize,
) -> Result<crate::model::LensOutputBlock, String> {
    content(app, move |app| {
        commands::get_response_block(
            webview,
            app.state::<AppState>(),
            operation_id,
            representation_id,
            block_index,
        )
    })
    .await
}

#[tauri::command]
pub async fn get_html_output<R: tauri::Runtime>(
    app: AppHandle<R>,
    webview: tauri::Webview<R>,
    operation_id: Uuid,
    representation_id: Uuid,
    resource_id: String,
) -> Result<String, String> {
    content(app, move |app| {
        commands::get_html_output(
            webview,
            app.state::<AppState>(),
            operation_id,
            representation_id,
            resource_id,
        )
    })
    .await
}

#[tauri::command]
pub async fn get_session_block<R: tauri::Runtime>(
    app: AppHandle<R>,
    webview: tauri::Webview<R>,
    request: crate::session_view::wire::BlockRequest,
) -> Result<crate::session_view::wire::BlockResponse, String> {
    content(app, move |app| {
        crate::session_view::wire::get_session_block(webview, app.state::<AppState>(), request)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saturated_settings_lane_does_not_queue_or_block_controls_or_content() {
        let app = tauri::test::mock_builder()
            .manage(crate::test_support::state())
            .build(crate::product_context())
            .unwrap();
        let permits = app
            .state::<AppState>()
            .store
            .command_workers
            .settings
            .clone()
            .try_acquire_many_owned(16)
            .unwrap();
        let error = configuration(app.handle().clone(), |_| -> Result<(), String> {
            panic!("rejected work must not run")
        })
        .await
        .unwrap_err();
        assert!(error.contains("Too many pending"));
        assert_eq!(
            blocking(app.handle().clone(), |_| Ok(17)).await.unwrap(),
            17
        );
        assert_eq!(content(app.handle().clone(), |_| Ok(23)).await.unwrap(), 23);
        drop(permits);
        configuration(app.handle().clone(), |_| Ok(()))
            .await
            .unwrap();
    }
}
