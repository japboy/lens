use crate::{
    agent,
    app_state::{
        clear_lens_operation, commit_initial_lens_context, commit_initial_lens_failure,
        commit_lens_context_refresh, emit_app_snapshot, next_revision, publish_lens_state,
        update_lens_state, update_lens_state_for_context, AgentRunKey, AppState,
        LensContextRefreshCommit, LensContextRefreshOutcome,
    },
    confirm_targets::{confirm_targets, ConfirmationHost, OPERATION_SUPERSEDED},
    lens::{
        target_id, LensInput, LensMediaPayload, LensMediaPlan, LensMediaRequest, LensMediaScope,
        LensTargetSet, MAX_LENS_TARGETS,
    },
    live_runtime,
    live_sync::LensAgentProjection,
    model::{
        AgentKind, AgentSelectionState, AppConfig, AppSnapshot, ExtractionQuality, LensFreshness,
        LensLiveState, LensMonitoringLifecycle, LensRefreshOutcome, LensSourceHealth, LensStage,
        LensState, LensTargetSelection, LensTargetSelectionItem, LensTargetSelectionStage,
        SelectedWindow, LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
    },
    platform::ImageCaptureLimits,
    prompt_template::AgentPromptTemplate,
    ui,
};
use std::{collections::BTreeMap, num::NonZeroU64, path::PathBuf, sync::Arc};
use tauri::{AppHandle, Manager, State};
use usecase::acquisition::ReadIssuer;
use usecase::context::{
    build_context, BlockingExecutor, BuiltContext, ContextBuildRequest, WindowAccessMode,
};
use usecase::state::ContextReadAuthority;
use uuid::Uuid;

/// Owns admission until the initial workflow transfers it to retained Lens state.
/// In particular, dropping an IPC future must close a pending native picker.
struct InitialOperationLease {
    selection: Arc<dyn port_platform::selection::TargetSelection>,
    operation_id: Uuid,
    retained: bool,
    reconcile: Option<Box<dyn Fn() -> bool + Send + Sync>>,
}

impl InitialOperationLease {
    fn open(
        selection: Arc<dyn port_platform::selection::TargetSelection>,
        operation_id: Uuid,
    ) -> Result<Self, String> {
        selection
            .open_operation(operation_id)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            selection,
            operation_id,
            retained: false,
            reconcile: None,
        })
    }

    fn retain(&mut self) {
        self.retained = true;
    }

    fn track_state<R: tauri::Runtime>(&mut self, app: AppHandle<R>) {
        let operation_id = self.operation_id;
        self.reconcile = Some(Box::new(move || {
            let Ok(current) = app.state::<AppState>().lens() else {
                return false;
            };
            if current.operation_id != Some(operation_id) {
                let _ = app.state::<AppState>().read_operations.close(operation_id);
                return false;
            }
            // A completed diagnostic context owns the source even if showing its
            // window failed after publication. The user may still Stop that context.
            if current.context.is_some() && current.target_set.is_some() {
                return true;
            }
            if matches!(current.stage, LensStage::Selecting | LensStage::Extracting) {
                // No reviewed-state handoff occurred. The workflow picker lease still
                // prevents another selection from starting; clear checks operation
                // identity again and atomically revokes its media and snapshot.
                if let Err(error) = clear_lens_operation(&app, operation_id) {
                    eprintln!("Unable to clear unfinished Lens selection: {error}");
                }
            }
            let _ = app.state::<AppState>().read_operations.close(operation_id);
            false
        }));
    }
}

impl Drop for InitialOperationLease {
    fn drop(&mut self) {
        if !self.retained {
            if self.reconcile.as_ref().is_some_and(|reconcile| reconcile()) {
                return;
            }
            if let Err(error) = self.selection.release_operation(self.operation_id) {
                eprintln!("Unable to release the unfinished Lens operation: {error}");
            }
        }
    }
}

struct DesktopBlockingExecutor<R: tauri::Runtime> {
    app: AppHandle<R>,
    authority: ContextReadAuthority,
    registered: bool,
}

impl<R: tauri::Runtime> DesktopBlockingExecutor<R> {
    fn admit(&self) -> Result<(), String> {
        let state = self.app.state::<AppState>();
        let snapshot = state
            .runtime
            .read()
            .map_err(|_| "application state lock is poisoned")?;
        let operation_id = match self.authority {
            ContextReadAuthority::Initial { operation_id }
            | ContextReadAuthority::Refresh { operation_id, .. } => operation_id,
        };
        if self.authority.admits(&snapshot.lens)
            && (!self.registered || state.read_operations.is_open(operation_id)?)
        {
            Ok(())
        } else {
            Err("Lens source read authority was revoked".into())
        }
    }
}

impl<R: tauri::Runtime> ReadIssuer for DesktopBlockingExecutor<R> {
    fn reserve(
        &self,
        target: port_platform::authority::TargetAuthority,
    ) -> Result<port_platform::authority::TargetReadKey, String> {
        if !self.registered {
            return Err("legacy diagnostic reads cannot issue registered read keys".into());
        }
        let state = self.app.state::<AppState>();
        let snapshot = state
            .runtime
            .read()
            .map_err(|_| "application state lock is poisoned")?;
        if !self.authority.admits(&snapshot.lens)
            || snapshot.lens.operation_id != Some(target.operation_id)
        {
            return Err("Lens source read authority was revoked".into());
        }
        state.read_operations.reserve(target)
    }
}

impl<R: tauri::Runtime> BlockingExecutor for DesktopBlockingExecutor<R> {
    fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, String>> + Send>> {
        if let Err(error) = self.admit() {
            return Box::pin(async move { Err(error) });
        }
        let executor = Self {
            app: self.app.clone(),
            authority: self.authority,
            registered: self.registered,
        };
        let task = tauri::async_runtime::spawn_blocking(move || {
            // A worker queued before Pause/Stop must recheck when it actually starts.
            executor.admit()?;
            Ok(work())
        });
        Box::pin(async move { task.await.map_err(|error| error.to_string())? })
    }
}

const MAX_SELECTION_PREVIEW_LONG_EDGE: u32 = 480;
const MAX_SELECTION_PREVIEW_PIXELS: u32 = 230_400;
const MAX_SELECTION_PREVIEW_BYTES: u32 = 1024 * 1024;

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    state.snapshot()
}

#[tauri::command]
pub fn get_html_output<R: tauri::Runtime>(
    webview: tauri::Webview<R>,
    state: State<'_, AppState>,
    operation_id: Uuid,
    representation_id: Uuid,
    resource_id: String,
) -> Result<String, String> {
    if webview.label() != "lens-overlay" {
        return Err("HTML output is only available to the Lens overlay".into());
    }
    html_output(
        &state.lens()?,
        operation_id,
        representation_id,
        &resource_id,
    )
}

fn html_output(
    lens: &LensState,
    operation_id: Uuid,
    representation_id: Uuid,
    resource_id: &str,
) -> Result<String, String> {
    let representation = lens
        .representation
        .as_ref()
        .filter(|representation| {
            lens.operation_id == Some(operation_id)
                && representation.representation_id == representation_id
        })
        .ok_or("HTML output representation is no longer available")?;
    representation
        .output_blocks
        .iter()
        .find_map(|block| match block {
            crate::model::LensOutputBlock::Html {
                resource_id: id,
                text,
                ..
            } if id == resource_id => Some(text.clone()),
            _ => None,
        })
        .ok_or_else(|| "HTML output resource is no longer available".into())
}

pub async fn select_agent<R: tauri::Runtime>(
    app: AppHandle<R>,
    candidate: AgentKind,
) -> Result<AgentSelectionState, String> {
    agent::select_agent(app, candidate).await
}

#[tauri::command]
pub async fn set_agent<R: tauri::Runtime>(
    app: AppHandle<R>,
    agent: AgentKind,
) -> Result<AgentSelectionState, String> {
    select_agent(app, agent).await
}

#[tauri::command]
pub fn set_working_directory<R: tauri::Runtime>(
    path: String,
    app: AppHandle<R>,
) -> Result<AppConfig, String> {
    update_working_directory(&app, PathBuf::from(path))
}

#[tauri::command]
pub fn set_agent_prompt_template<R: tauri::Runtime>(
    agent_prompt_template: AgentPromptTemplate,
    app: AppHandle<R>,
) -> Result<AppConfig, String> {
    let agent_prompt_template = agent_prompt_template.normalize()?;
    update_config(&app, |config| {
        config.agent_prompt_template = agent_prompt_template
    })
}

#[tauri::command]
pub fn reset_agent_prompt_template<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<AppConfig, String> {
    update_config(&app, |config| {
        config.agent_prompt_template = AgentPromptTemplate::default();
    })
}

pub fn update_working_directory<R: tauri::Runtime>(
    app: &AppHandle<R>,
    directory: PathBuf,
) -> Result<AppConfig, String> {
    if !directory.is_absolute() || !directory.is_dir() {
        return Err("working directory must be an existing absolute directory".into());
    }
    update_config(app, |config| config.working_directory = directory)
}

fn update_config<R: tauri::Runtime>(
    app: &AppHandle<R>,
    update: impl FnOnce(&mut AppConfig),
) -> Result<AppConfig, String> {
    let state = app.state::<AppState>();
    let _ = state.agent_control.cancel_active()?;
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
pub fn accessibility_permission(
    state: State<'_, AppState>,
) -> usecase::platform::AccessibilityAccessReply {
    usecase::platform::accessibility_access(state.platform.trust.inspect())
}

#[tauri::command]
pub fn request_accessibility_permission(
    state: State<'_, AppState>,
) -> usecase::platform::AccessibilityAccessReply {
    usecase::platform::request_accessibility_access(state.platform.trust.as_ref())
}

async fn select_single_window<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
) -> Result<Option<SelectedWindow>, String> {
    let state = app.state::<AppState>();
    let reply = state
        .platform
        .selection
        .pick(operation_id)
        .await
        .map_err(|error| error.to_string())?;
    let Some(mut windows) = usecase::platform::picker_reply(reply).into_selected()? else {
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

pub async fn select_and_extract<R: tauri::Runtime>(app: AppHandle<R>) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let _picker_lease = state.picker_control.try_begin()?;
    let previous = state.lens()?;
    if previous.live.is_some() {
        return Err("stop the active Lens before selecting another target set".into());
    }
    let _ = state.agent_control.cancel_active()?;
    let operation_id = Uuid::new_v4();
    let mut operation =
        InitialOperationLease::open(state.platform.selection.clone(), operation_id)?;
    operation.track_state(app.clone());
    if let Some(previous_id) = previous.operation_id {
        state.read_operations.close(previous_id)?;
        state
            .platform
            .selection
            .release_operation(previous_id)
            .map_err(|error| error.to_string())?;
    }
    state.read_operations.open(operation_id)?;
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
        match select_single_window(&app, operation_id).await {
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
    let result = extract_target_set_for_operation(
        app.clone(),
        operation_id,
        target_set,
        WindowAccessMode::Registered,
    )
    .await?;
    if app.state::<AppState>().lens()?.operation_id != Some(operation_id) {
        return Err(OPERATION_SUPERSEDED.into());
    }
    operation.retain();
    Ok(result)
}

pub async fn extract_target<R: tauri::Runtime>(
    app: AppHandle<R>,
    target: SelectedWindow,
    window_id: u32,
    pid: i32,
) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let _ = state.agent_control.cancel_active()?;
    let operation_id = target.identity.operation_id;
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
    extract_target_set_for_operation(
        app,
        operation_id,
        target_set,
        WindowAccessMode::Legacy { window_id, pid },
    )
    .await
}

async fn extract_target_set_for_operation<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    target_set: LensTargetSet,
    access_mode: WindowAccessMode,
) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let _read_workflow = state
        .read_workflow
        .try_lock()
        .map_err(|_| "Lens source read is already active")?;
    let context_revision = 1;
    let source_revisions = target_set
        .targets
        .iter()
        .map(|target| (target.id.clone(), 1))
        .collect::<BTreeMap<_, _>>();
    let BuiltContext {
        target_set: refreshed_target_set,
        context,
        payloads,
    } = match build_context(
        &DesktopBlockingExecutor {
            app: app.clone(),
            authority: ContextReadAuthority::Initial { operation_id },
            registered: access_mode == WindowAccessMode::Registered,
        },
        &DesktopBlockingExecutor {
            app: app.clone(),
            authority: ContextReadAuthority::Initial { operation_id },
            registered: access_mode == WindowAccessMode::Registered,
        },
        app.state::<AppState>().platform.accessibility.clone(),
        app.state::<AppState>().platform.capture.clone(),
        ContextBuildRequest {
            operation_id,
            context_revision,
            source_revisions,
            target_set: &target_set,
            access_mode,
        },
    )
    .await
    {
        Ok(context) => context,
        Err(message) => {
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
    let projection = input
        .as_ref()
        .map(|input| LensAgentProjection::from_input(input, &refreshed_target_set, &payloads))
        .transpose()
        .map_err(|error| error.to_string())?;
    let projection_ref = projection.as_ref().map(|projection| {
        projection.projection_ref(NonZeroU64::new(1).expect("initial revision is non-zero"))
    });
    let has_input = input.is_some();
    let live = (access_mode == WindowAccessMode::Registered).then_some(LensLiveState {
        lifecycle: LensMonitoringLifecycle::Watching,
        health: source_health(context.quality),
        freshness: LensFreshness::None,
        agent_refresh_interval_seconds: LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
        last_outcome: None,
        error: None,
    });
    let next = LensState {
        session_controls: None,
        operation_id: Some(operation_id),
        stage: if input.is_some() {
            LensStage::Ready
        } else {
            LensStage::Failed
        },
        selection: None,
        target_set: Some(refreshed_target_set.clone()),
        context: Some(context),
        input,
        projection: projection_ref,
        output_blocks: Vec::new(),
        representation: None,
        pending_representation: None,
        live,
        agent: None,
        error,
    };
    if has_input {
        if !commit_initial_lens_context(&app, operation_id, next.clone(), payloads)? {
            return Err(OPERATION_SUPERSEDED.into());
        }
    } else if !commit_initial_lens_failure(&app, operation_id, next.clone(), payloads)? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    if let Err(error) = ui::show_lens_window(&app, &refreshed_target_set) {
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

pub(crate) async fn refresh_lens_context<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    expected_context_revision: u64,
) -> Result<LensContextRefreshOutcome, String> {
    let state = app.state::<AppState>();
    let _read_workflow = state
        .read_workflow
        .try_lock()
        .map_err(|_| "Lens source read is already active")?;
    let current = app.state::<AppState>().lens()?;
    let context = current
        .context
        .as_ref()
        .filter(|context| {
            current.operation_id == Some(operation_id)
                && context.revision == expected_context_revision
        })
        .ok_or_else(|| "Lens context refresh was superseded before it started".to_string())?;
    let context_id = context.context_id;
    if current
        .live
        .as_ref()
        .is_none_or(|live| live.lifecycle != LensMonitoringLifecycle::Watching)
    {
        return Err(
            "Lens context refresh is not allowed while monitoring is paused or stopped".into(),
        );
    }
    let target_set = current
        .target_set
        .clone()
        .ok_or_else(|| "the current Lens operation has no fixed target set".to_string())?;
    let next_context_revision = expected_context_revision
        .checked_add(1)
        .ok_or_else(|| "Lens context revision is exhausted".to_string())?;
    let source_revisions = context
        .sources
        .iter()
        .map(|source| {
            source
                .revision
                .checked_add(1)
                .map(|revision| (source.target_id.clone(), revision))
                .ok_or_else(|| {
                    format!("Lens source revision is exhausted for {}", source.target_id)
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    if !update_lens_state_for_context(
        &app,
        operation_id,
        context_id,
        expected_context_revision,
        mark_context_refresh_started,
    )? {
        return Err("Lens context refresh was superseded before extraction".into());
    }

    let BuiltContext {
        target_set: next_target_set,
        context: next_context,
        payloads,
    } = match build_context(
        &DesktopBlockingExecutor {
            app: app.clone(),
            authority: ContextReadAuthority::Refresh {
                operation_id,
                context_id,
                previous_revision: expected_context_revision,
            },
            registered: true,
        },
        &DesktopBlockingExecutor {
            app: app.clone(),
            authority: ContextReadAuthority::Refresh {
                operation_id,
                context_id,
                previous_revision: expected_context_revision,
            },
            registered: true,
        },
        app.state::<AppState>().platform.accessibility.clone(),
        app.state::<AppState>().platform.capture.clone(),
        ContextBuildRequest {
            operation_id,
            context_revision: next_context_revision,
            source_revisions,
            target_set: &target_set,
            access_mode: WindowAccessMode::Registered,
        },
    )
    .await
    {
        Ok(built) => built,
        Err(error) => {
            mark_refresh_failed(
                &app,
                operation_id,
                context_id,
                expected_context_revision,
                error.clone(),
            )?;
            return Err(error);
        }
    };
    let Some(next_input) = LensInput::from_context(&next_context) else {
        let error = if next_context.diagnostics.is_empty() {
            "Selected-window refresh did not yield usable structured content or media.".into()
        } else {
            next_context.diagnostics.join("; ")
        };
        mark_refresh_failed(
            &app,
            operation_id,
            context_id,
            expected_context_revision,
            error.clone(),
        )?;
        return Err(error);
    };
    let candidate = match LensAgentProjection::from_input(&next_input, &next_target_set, &payloads)
    {
        Ok(candidate) => candidate,
        Err(error) => {
            let error = error.to_string();
            mark_refresh_failed(
                &app,
                operation_id,
                context_id,
                expected_context_revision,
                error.clone(),
            )?;
            return Err(error);
        }
    };
    let previous_projection = current
        .projection
        .as_ref()
        .ok_or_else(|| "the current Lens operation has no Agent projection".to_string())?;
    let changed = candidate.digest() != &previous_projection.digest;
    let next_projection = if changed {
        let revision = previous_projection
            .revision
            .get()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or_else(|| "Lens Agent projection revision is exhausted".to_string())?;
        candidate.projection_ref(revision)
    } else {
        previous_projection.clone()
    };

    let outcome = if changed {
        LensContextRefreshOutcome::Updated
    } else {
        LensContextRefreshOutcome::Unchanged
    };
    let next_source_health = source_health(next_context.quality);
    if !commit_lens_context_refresh(
        &app,
        operation_id,
        context_id,
        expected_context_revision,
        LensContextRefreshCommit {
            target_set: next_target_set,
            context: next_context,
            input: next_input,
            projection: next_projection,
            source_health: next_source_health,
            outcome,
        },
        payloads,
    )? {
        return Err("Lens context refresh was superseded before canonical commit".into());
    }
    Ok(outcome)
}

fn mark_context_refresh_started(lens: &mut LensState) {
    if matches!(
        lens.stage,
        LensStage::AuthenticationRequired | LensStage::Cancelled | LensStage::Failed
    ) {
        return;
    }
    if let Some(live) = lens.live.as_mut() {
        live.freshness = if live.health == LensSourceHealth::Unavailable {
            LensFreshness::Unverified
        } else {
            LensFreshness::Checking
        };
        live.last_outcome = None;
        live.error = None;
    }
}

fn mark_refresh_failed<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    context_id: Uuid,
    context_revision: u64,
    error: String,
) -> Result<(), String> {
    update_lens_state_for_context(app, operation_id, context_id, context_revision, |lens| {
        if let Some(live) = lens.live.as_mut() {
            live.health = LensSourceHealth::Unavailable;
            live.freshness = LensFreshness::Unverified;
            live.last_outcome = Some(LensRefreshOutcome::Failed);
            live.error = Some(error);
        }
    })?;
    Ok(())
}

fn source_health(quality: ExtractionQuality) -> LensSourceHealth {
    match quality {
        ExtractionQuality::Full => LensSourceHealth::Healthy,
        ExtractionQuality::Partial => LensSourceHealth::Degraded,
        ExtractionQuality::Unavailable => LensSourceHealth::Unavailable,
    }
}

async fn build_target_selection_item<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    window: SelectedWindow,
) -> Result<(LensTargetSelectionItem, Option<LensMediaPayload>), String> {
    let state = app.state::<AppState>();
    let _read_workflow = state
        .read_workflow
        .try_lock()
        .map_err(|_| "Lens source read is already active")?;
    let read = {
        let snapshot = state
            .runtime
            .read()
            .map_err(|_| "application state lock is poisoned")?;
        if snapshot.lens.operation_id != Some(operation_id)
            || snapshot.lens.stage != LensStage::Selecting
            || !snapshot
                .lens
                .selection
                .as_ref()
                .is_some_and(|selection| selection.stage == LensTargetSelectionStage::Picking)
            || window.identity.operation_id != operation_id
        {
            return Err("Lens preview read authority was revoked".into());
        }
        state
            .read_operations
            .reserve(port_platform::authority::TargetAuthority {
                operation_id,
                receipt: window.identity.receipt.try_into().map_err(str::to_owned)?,
            })?
    };
    let capture_service = state.platform.capture.clone();
    let id = target_id(&window);
    let attachment_id = format!("selection-preview-window-{}", window.identity.receipt);
    let plan = LensMediaPlan {
        requests: vec![LensMediaRequest {
            id: attachment_id,
            scope: LensMediaScope::WindowFallback,
            source_node_id: None,
            bounds: None,
        }],
        omissions: Vec::new(),
    };
    let capture_target_id = id.clone();
    let worker_app = app.clone();
    let capture = tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app.state::<AppState>();
        let snapshot = state.runtime.read().map_err(|_| {
            port_platform::PlatformError::Operation("application state lock is poisoned".into())
        })?;
        if snapshot.lens.operation_id != Some(operation_id)
            || snapshot.lens.stage != LensStage::Selecting
            || !state
                .read_operations
                .is_open(operation_id)
                .map_err(port_platform::PlatformError::Operation)?
        {
            return Err(port_platform::PlatformError::Operation(
                "Lens preview read authority was revoked".into(),
            ));
        }
        drop(snapshot);
        usecase::media::capture_media(
            capture_service.as_ref(),
            usecase::media::MediaCaptureContext {
                geometry: port_platform::geometry::CaptureGeometryExpectation::ObserveCurrent,
                target: port_platform::capture::CaptureTarget::Registered { read },
                target_id: capture_target_id,
                context_id: operation_id,
                context_revision: NonZeroU64::MIN,
            },
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
                window.identity.receipt
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

    Ok((
        LensTargetSelectionItem {
            id,
            window,
            preview_uri,
            preview_error,
        },
        payload,
    ))
}

fn target_selection_for_operation<R: tauri::Runtime>(
    app: &AppHandle<R>,
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
        || selection.anchor.is_some() == selection.items.is_empty()
    {
        return Err("Lens target selection state violates its versioned invariants".into());
    }
    Ok(selection)
}

async fn publish_target_selection<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    selection: LensTargetSelection,
) -> Result<LensState, String> {
    let next = LensState {
        session_controls: None,
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

fn store_target_selection_payload<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    payload: Option<LensMediaPayload>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snapshot = state
        .runtime
        .read()
        .map_err(|_| "application state lock is poisoned")?;
    if snapshot.lens.operation_id != Some(operation_id)
        || snapshot.lens.stage != LensStage::Selecting
        || !state.read_operations.is_open(operation_id)?
    {
        return Err(OPERATION_SUPERSEDED.into());
    }
    let mut payloads = state.lens_media.payloads(operation_id)?;
    if let Some(payload) = payload {
        payloads.push(payload);
    }
    if !state.lens_media.replace(operation_id, payloads)? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    Ok(())
}

struct AddSelectionLease<R: tauri::Runtime> {
    app: AppHandle<R>,
    operation_id: Uuid,
    accepted_window: Option<Uuid>,
    preview_uri: Option<String>,
}

impl<R: tauri::Runtime> Drop for AddSelectionLease<R> {
    fn drop(&mut self) {
        let state = self.app.state::<AppState>();
        let outcome = (|| -> Result<bool, String> {
            let mut snapshot = state
                .runtime
                .write()
                .map_err(|_| "application state lock is poisoned")?;
            let owned = snapshot.lens.operation_id == Some(self.operation_id)
                && self.accepted_window.is_some_and(|id| {
                    snapshot.lens.selection.as_ref().is_some_and(|selection| {
                        selection
                            .items
                            .iter()
                            .any(|item| item.window.identity.receipt == id)
                    }) || snapshot.lens.target_set.as_ref().is_some_and(|set| {
                        set.targets
                            .iter()
                            .any(|target| target.identity.receipt == id)
                    })
                });
            let restore =
                snapshot.lens.operation_id == Some(self.operation_id)
                    && state.read_operations.is_open(self.operation_id)?
                    && snapshot.lens.stage == LensStage::Selecting
                    && snapshot.lens.selection.as_ref().is_some_and(|selection| {
                        selection.stage == LensTargetSelectionStage::Picking
                    });
            let notification = if restore {
                let revision = next_revision(&snapshot)?;
                snapshot
                    .lens
                    .selection
                    .as_mut()
                    .expect("checked selection")
                    .stage = LensTargetSelectionStage::Reviewing;
                snapshot.revision = revision;
                Some(snapshot.clone())
            } else {
                None
            };
            drop(snapshot);
            if let Some(snapshot) = notification {
                if let Err(error) = emit_app_snapshot(&self.app, snapshot, false) {
                    eprintln!("Unable to notify target addition rollback: {error}");
                }
            }
            Ok(owned)
        })();
        match outcome {
            Ok(false) => {
                if let Some(id) = self.accepted_window {
                    if let Err(error) = state.platform.selection.release_target(
                        self.operation_id,
                        match id.try_into() {
                            Ok(receipt) => receipt,
                            Err(error) => {
                                eprintln!("Invalid added target receipt: {error}");
                                return;
                            }
                        },
                    ) {
                        eprintln!("Unable to release unfinished added target: {error}");
                    }
                }
                if let Some(uri) = &self.preview_uri {
                    let _ = state
                        .lens_media
                        .remove_uri_for_operation(self.operation_id, uri);
                }
            }
            Ok(true) => {}
            Err(error) => eprintln!("Unable to reconcile unfinished target addition: {error}"),
        }
    }
}

async fn publish_added_selection<R: tauri::Runtime>(
    app: &AppHandle<R>,
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
    // Publication transfers target ownership before the cancellable presentation await.
    // A presentation error must not erase the already reviewed target set.
    ui::show_target_selection_window(app, &selection)
        .await
        .map_err(|error| error.to_string())?;
    if app.state::<AppState>().lens()?.operation_id != Some(operation_id) {
        return Err(OPERATION_SUPERSEDED.into());
    }
    Ok(next)
}

#[tauri::command]
pub async fn select_lens_target<R: tauri::Runtime>(app: AppHandle<R>) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let _picker_lease = state.picker_control.try_begin()?;
    let agent_selection = state.agent_selection()?;
    if !agent_selection.can_select_lens_target() {
        return Err("select and authenticate an AI Agent before selecting a Lens Target".into());
    }
    let previous = state.lens()?;
    if previous.live.is_some() {
        return Err("stop the active Lens before selecting another target set".into());
    }
    let _ = state.agent_control.cancel_active()?;
    let operation_id = Uuid::new_v4();
    let mut operation =
        InitialOperationLease::open(state.platform.selection.clone(), operation_id)?;
    operation.track_state(app.clone());
    if let Some(previous_id) = previous.operation_id {
        state.read_operations.close(previous_id)?;
        state
            .platform
            .selection
            .release_operation(previous_id)
            .map_err(|error| error.to_string())?;
    }
    state.read_operations.open(operation_id)?;
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

    let window = match select_single_window(&app, operation_id).await {
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
    let anchor = window.facts.frame;
    let (item, payload) = build_target_selection_item(app.clone(), operation_id, window).await?;
    store_target_selection_payload(&app, operation_id, payload)?;
    let reviewed = LensTargetSelection {
        selection_id: operation_id,
        stage: LensTargetSelectionStage::Reviewing,
        maximum_targets: MAX_LENS_TARGETS,
        anchor: Some(anchor),
        items: vec![item],
        notice: None,
    };
    let selected = LensState {
        operation_id: Some(operation_id),
        stage: LensStage::Selecting,
        selection: Some(reviewed.clone()),
        ..LensState::default()
    };
    if !replace_operation_state(&app, operation_id, selected.clone())? {
        return Err(OPERATION_SUPERSEDED.into());
    }
    // Commit transfers native ownership before presentation can suspend or fail.
    // An interrupted UI effect must not invalidate already published reviewed items.
    operation.retain();
    ui::show_target_selection_window(&app, &reviewed)
        .await
        .map_err(|error| error.to_string())?;
    if app.state::<AppState>().lens()?.operation_id != Some(operation_id) {
        return Err(OPERATION_SUPERSEDED.into());
    }
    Ok(selected)
}

#[tauri::command]
pub async fn add_lens_target<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<LensState, String> {
    let state = app.state::<AppState>();
    let _picker_lease = state.picker_control.try_begin()?;
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
    let existing_windows: Vec<_> = selection
        .items
        .iter()
        .map(|item| item.window.identity.receipt)
        .collect();
    let mut addition = AddSelectionLease {
        app: app.clone(),
        operation_id,
        accepted_window: None,
        preview_uri: None,
    };
    selection.stage = LensTargetSelectionStage::Picking;
    selection.notice = None;
    publish_added_selection(&app, operation_id, selection.clone()).await?;

    let selected = select_single_window(&app, operation_id).await;
    if let Ok(Some(window)) = &selected {
        if !existing_windows.contains(&window.identity.receipt) {
            addition.accepted_window = Some(window.identity.receipt);
        }
    }
    selection = target_selection_for_operation(&app, operation_id)?;
    match selected {
        Ok(None) => {
            selection.stage = LensTargetSelectionStage::Reviewing;
            publish_added_selection(&app, operation_id, selection).await
        }
        Err(error) => {
            selection.stage = LensTargetSelectionStage::Reviewing;
            selection.notice = Some(format!("Unable to add a window: {error}"));
            publish_added_selection(&app, operation_id, selection).await?;
            Err(error)
        }
        Ok(Some(window)) => {
            if selection
                .items
                .iter()
                .any(|item| item.window.identity.receipt == window.identity.receipt)
            {
                selection.stage = LensTargetSelectionStage::Reviewing;
                selection.notice = Some("That window is already selected.".into());
                return publish_added_selection(&app, operation_id, selection).await;
            }
            let (item, payload) =
                build_target_selection_item(app.clone(), operation_id, window).await?;
            addition.preview_uri = payload.as_ref().map(|payload| payload.uri.clone());
            store_target_selection_payload(&app, operation_id, payload)?;
            selection.items.push(item);
            selection.stage = LensTargetSelectionStage::Reviewing;
            selection.notice = None;
            publish_added_selection(&app, operation_id, selection).await
        }
    }
}

#[tauri::command]
pub async fn remove_lens_target<R: tauri::Runtime>(
    app: AppHandle<R>,
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
    let receipt = selection.items[index].window.identity.receipt;
    // Release native ownership before mutating the authoritative Rust media/state snapshot. A
    // native teardown failure therefore leaves the reviewed item intact and retryable.
    app.state::<AppState>()
        .platform
        .selection
        .release_target(
            operation_id,
            receipt
                .try_into()
                .map_err(|error: &str| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
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
pub async fn confirm_lens_targets<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<LensState, String> {
    confirm_targets(&DesktopConfirmation { app }, operation_id).await
}

struct DesktopConfirmation<R: tauri::Runtime> {
    app: AppHandle<R>,
}

impl<R: tauri::Runtime> ConfirmationHost for DesktopConfirmation<R> {
    fn selection(&self, operation: Uuid) -> Result<LensTargetSelection, String> {
        target_selection_for_operation(&self.app, operation)
    }

    async fn dismiss_preview(&self) -> Result<(), String> {
        ui::dismiss_target_selection_window(&self.app)
            .await
            .map_err(|error| error.to_string())
    }

    fn replace_state(&self, operation: Uuid, next: LensState) -> Result<bool, String> {
        replace_operation_state(&self.app, operation, next)
    }

    async fn extract_registered(
        &self,
        operation: Uuid,
        targets: LensTargetSet,
    ) -> Result<LensState, String> {
        extract_target_set_for_operation(
            self.app.clone(),
            operation,
            targets,
            WindowAccessMode::Registered,
        )
        .await
    }

    fn start_observation(&self, operation: Uuid) -> Result<(), String> {
        live_runtime::start(&self.app, operation)
    }

    fn mark_observation_unavailable(&self, operation: Uuid, error: String) -> Result<(), String> {
        update_lens_state(&self.app, operation, |lens| {
            if let Some(live) = lens.live.as_mut() {
                live.health = LensSourceHealth::Unavailable;
                live.freshness = LensFreshness::Unverified;
                live.error = Some(error.clone());
            }
        })?;
        Ok(())
    }

    async fn transform(&self, operation: Uuid) -> Result<LensState, String> {
        agent::transform_current(self.app.clone(), operation).await
    }

    fn request_refresh(&self, operation: Uuid) {
        let _ = live_runtime::request_immediate_refresh(&self.app, operation);
    }
}

#[tauri::command]
pub async fn retry_lens_transform<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<LensState, String> {
    let lens = app.state::<AppState>().lens()?;
    if lens.operation_id != Some(operation_id) {
        return Err(OPERATION_SUPERSEDED.into());
    }
    if !can_retry_agent_transform(lens.stage, lens.input.is_some()) {
        return Err("the current Lens operation is not waiting for an Agent retry".into());
    }
    agent::transform_current(app, operation_id).await
}

fn can_retry_agent_transform(stage: LensStage, has_input: bool) -> bool {
    has_input && matches!(stage, LensStage::AuthenticationRequired | LensStage::Failed)
}

#[tauri::command]
pub fn pause_lens<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<LensState, String> {
    live_runtime::pause(&app, operation_id)
}

#[tauri::command]
pub fn resume_lens<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<LensState, String> {
    live_runtime::resume(&app, operation_id)
}

#[tauri::command]
pub fn stop_lens<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
) -> Result<LensState, String> {
    let current = app.state::<AppState>().lens()?;
    if current.operation_id != Some(operation_id) {
        return Err("Lens operation was superseded".into());
    }
    let stopped = {
        let state = app.state::<AppState>();
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned")?;
        if snapshot.lens.operation_id != Some(operation_id) {
            return Err(OPERATION_SUPERSEDED.into());
        }
        let revision = next_revision(&snapshot)?;
        state.read_operations.close(operation_id)?;
        let lens = &mut snapshot.lens;
        lens.stage = LensStage::Cancelled;
        if let Some(live) = lens.live.as_mut() {
            live.lifecycle = LensMonitoringLifecycle::Stopped;
            live.freshness = LensFreshness::Unverified;
        }
        lens.pending_representation = None;
        snapshot.revision = revision;
        snapshot.clone()
    };
    emit_app_snapshot(&app, stopped, false)?;
    live_runtime::stop(&app, operation_id)?;
    // Closing an absent native operation is idempotent, including legacy reads.
    // Do not infer native admission from a presentation stage: Stop can race
    // initial picking or extraction before selection/live fields are published.
    if let Err(error) = app
        .state::<AppState>()
        .platform
        .selection
        .release_operation(operation_id)
    {
        eprintln!("Unable to release the stopped native Lens operation: {error}");
    }
    if !clear_lens_operation(&app, operation_id)? {
        return Err("Lens operation was superseded before it stopped".into());
    }
    app.state::<AppState>().lens()
}

#[tauri::command]
pub async fn authenticate_agent<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    method_id: String,
) -> Result<LensState, String> {
    agent::authenticate_current(app, operation_id, method_id).await
}

#[tauri::command]
pub async fn authenticate_agent_selection<R: tauri::Runtime>(
    app: AppHandle<R>,
    method_id: String,
) -> Result<AgentSelectionState, String> {
    agent::authenticate_selection(app, method_id).await
}

#[tauri::command]
pub async fn reauthenticate_agent_selection<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<AgentSelectionState, String> {
    agent::reauthenticate_selection(app).await
}

#[tauri::command]
pub async fn sign_out_agent_selection<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<AgentSelectionState, String> {
    agent::sign_out_selection(app).await
}

#[tauri::command]
pub fn cancel_agent<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    run_id: Uuid,
) -> Result<LensState, String> {
    agent::cancel_current(
        &app,
        AgentRunKey {
            operation_id,
            run_id,
        },
    )
}

#[tauri::command]
pub fn show_settings<R: tauri::Runtime>(app: AppHandle<R>) -> Result<(), String> {
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

fn replace_operation_state<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    next: LensState,
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let (snapshot, sync_tray) = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "application state lock is poisoned")?;
        if snapshot.lens.operation_id != Some(operation_id)
            || snapshot.lens.stage == LensStage::Cancelled
            || ((next.selection.is_some() || next.live.is_some())
                && !state.read_operations.is_open(operation_id)?)
        {
            return Ok(false);
        }
        let sync_tray = snapshot.lens.live.is_some() != next.live.is_some();
        snapshot.revision = next_revision(&snapshot)?;
        snapshot.lens = next;
        (snapshot.clone(), sync_tray)
    };
    emit_app_snapshot(app, snapshot, sync_tray)?;
    Ok(true)
}

/// Resolve model-dependent settings without persisting a draft or touching the live actor.
#[tauri::command]
pub async fn preview_agent_model<R: tauri::Runtime>(
    app: AppHandle<R>,
    selection_id: Uuid,
    config_id: String,
    value: Option<String>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let expected = state.snapshot()?;
    if expected.agent_selection.operation_id != Some(selection_id)
        || expected.agent_selection.selected_agent() != Some(expected.config.agent)
        || !expected.agent_selection.config_options.as_ref().is_some_and(|options| options.iter().any(|o|
            o.id.to_string() == config_id && o.category == Some(agent_client_protocol::schema::v1::SessionConfigOptionCategory::Model)))
    { return Err("Agent model selection changed".into()); }
    let defaults = crate::agent_preferences::AgentDefaults {
        choices: value
            .map(|value| crate::agent_preferences::SavedChoice { config_id, value })
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let options = agent::validate_agent_defaults(&app, &expected.config, &defaults).await?;
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "Application state is unavailable")?;
        if snapshot.config != expected.config
            || snapshot.agent_selection != expected.agent_selection
        {
            return Err("Agent settings changed during model lookup".into());
        }
        snapshot.revision = next_revision(&snapshot)?;
        snapshot.agent_selection.config_options = options;
        snapshot.clone()
    };
    emit_app_snapshot(&app, snapshot, true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn registered_reads_require_live_issuer_and_stop_does_not_wait_for_workflow() {
        let state = crate::test_support::state();
        let operation_id = Uuid::from_u128(1);
        state.runtime.write().unwrap().lens = LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Extracting,
            ..LensState::default()
        };
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(crate::product_context())
            .unwrap();
        let executor = DesktopBlockingExecutor {
            app: app.handle().clone(),
            authority: ContextReadAuthority::Initial { operation_id },
            registered: true,
        };
        assert!(executor.admit().is_err());
        let state = app.state::<AppState>();
        state.read_operations.open(operation_id).unwrap();
        assert!(executor.admit().is_ok());
        let target = port_platform::authority::TargetAuthority {
            operation_id,
            receipt: Uuid::from_u128(2).try_into().unwrap(),
        };
        assert!(executor.reserve(target).is_ok());
        let _workflow = state.read_workflow.try_lock().unwrap();
        assert!(state.read_workflow.try_lock().is_err());
        state.read_operations.close(operation_id).unwrap();
        assert!(executor.admit().is_err());
        assert!(executor.reserve(target).is_err());
        let legacy = DesktopBlockingExecutor {
            registered: false,
            ..executor
        };
        assert!(legacy.admit().is_ok());
        assert!(legacy.reserve(target).is_err());
    }

    use super::*;
    use crate::prompt_template::MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS;

    struct OperationFixture {
        effects: std::sync::Mutex<Vec<&'static str>>,
        fail_open: bool,
    }

    impl port_platform::selection::TargetSelection for OperationFixture {
        fn open_operation(&self, _: Uuid) -> Result<(), port_platform::PlatformError> {
            self.effects.lock().unwrap().push("open");
            if self.fail_open {
                Err(port_platform::PlatformError::Operation(
                    "open rejected".into(),
                ))
            } else {
                Ok(())
            }
        }
        fn pick(
            &self,
            _: Uuid,
        ) -> port_platform::PlatformFuture<
            '_,
            Result<port_platform::selection::WindowPickerReply, port_platform::PlatformError>,
        > {
            panic!("lease tests do not invoke native picking")
        }
        fn release_target(
            &self,
            _: Uuid,
            _: port_platform::authority::TargetReceipt,
        ) -> Result<(), port_platform::PlatformError> {
            self.effects.lock().unwrap().push("release_target");
            Ok(())
        }
        fn release_operation(&self, _: Uuid) -> Result<(), port_platform::PlatformError> {
            self.effects.lock().unwrap().push("release");
            Ok(())
        }
    }

    #[test]
    fn initial_operation_lease_closes_on_drop_but_not_after_handoff() {
        let fixture = Arc::new(OperationFixture {
            effects: Default::default(),
            fail_open: false,
        });
        let lease = InitialOperationLease::open(fixture.clone(), Uuid::new_v4()).unwrap();
        assert_eq!(*fixture.effects.lock().unwrap(), ["open"]);
        drop(lease);
        assert_eq!(*fixture.effects.lock().unwrap(), ["open", "release"]);
        let mut lease = InitialOperationLease::open(fixture.clone(), Uuid::new_v4()).unwrap();
        lease.retain();
        drop(lease);
        assert_eq!(
            *fixture.effects.lock().unwrap(),
            ["open", "release", "open"]
        );
    }

    #[test]
    fn rejected_initial_open_does_not_release_an_operation_it_does_not_own() {
        let fixture = Arc::new(OperationFixture {
            effects: Default::default(),
            fail_open: true,
        });
        assert!(InitialOperationLease::open(fixture.clone(), Uuid::new_v4()).is_err());
        assert_eq!(*fixture.effects.lock().unwrap(), ["open"]);
    }

    #[test]
    fn cancelling_pending_initial_workflow_releases_native_admission() {
        let fixture = Arc::new(OperationFixture {
            effects: Default::default(),
            fail_open: false,
        });
        let source = fixture.clone();
        let mut workflow = Box::pin(async move {
            let _operation = InitialOperationLease::open(source, Uuid::new_v4()).unwrap();
            std::future::pending::<()>().await;
        });
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(workflow.as_mut(), &mut context).is_pending());
        assert_eq!(*fixture.effects.lock().unwrap(), ["open"]);
        drop(workflow);
        assert_eq!(*fixture.effects.lock().unwrap(), ["open", "release"]);
    }

    #[test]
    fn unfinished_initial_state_is_cleared_without_touching_a_replacement_operation() {
        struct CleanupTray(Arc<std::sync::atomic::AtomicUsize>);
        impl crate::ui::TrayOutput<tauri::test::MockRuntime> for CleanupTray {
            fn apply(
                &self,
                _: &AppHandle<tauri::test::MockRuntime>,
                _: crate::ui::TrayMenuPresentation,
            ) -> Result<(), String> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
        for superseded in [false, true] {
            let tray_updates = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let fixture = Arc::new(OperationFixture {
                effects: Default::default(),
                fail_open: false,
            });
            let state = crate::test_support::state();
            let operation_id = Uuid::new_v4();
            let current_id = if superseded {
                Uuid::new_v4()
            } else {
                operation_id
            };
            state.lens_media.begin(current_id).unwrap();
            state.runtime.write().unwrap().lens = LensState {
                operation_id: Some(current_id),
                stage: LensStage::Selecting,
                ..LensState::default()
            };
            let app = tauri::test::mock_builder()
                .manage(state)
                .manage(crate::ui::TrayPresentation(Arc::new(CleanupTray(
                    tray_updates.clone(),
                ))))
                .build(crate::product_context())
                .unwrap();
            let mut lease = InitialOperationLease::open(fixture.clone(), operation_id).unwrap();
            lease.track_state(app.handle().clone());
            drop(lease);
            assert_eq!(*fixture.effects.lock().unwrap(), ["open", "release"]);
            assert_eq!(
                app.state::<AppState>().lens().unwrap().operation_id,
                superseded.then_some(current_id)
            );
            assert_eq!(
                app.state::<AppState>()
                    .lens_media
                    .payloads(current_id)
                    .is_ok(),
                superseded
            );
            assert_eq!(
                tray_updates.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(!superseded)
            );
        }
    }

    #[test]
    fn cancelled_addition_restores_latest_selection_and_releases_only_unpublished_target() {
        for published in [false, true] {
            let fixture = Arc::new(OperationFixture {
                effects: Default::default(),
                fail_open: false,
            });
            let mut state = crate::test_support::state();
            state.platform.selection = fixture.clone();
            let operation_id = Uuid::new_v4();
            state.read_operations.open(operation_id).unwrap();
            let window = SelectedWindow {
                identity: crate::model::WindowIdentity {
                    operation_id,
                    receipt: Uuid::from_u128(42),
                    selection_ordinal: 1,
                },
                facts: crate::model::WindowObservableFacts {
                    application_id: "example.fixture".into(),
                    title: "Fixture".into(),
                    application_name: "Fixture".into(),
                    frame: crate::model::Bounds {
                        x: 0.0,
                        y: 0.0,
                        width: 10.0,
                        height: 10.0,
                    },
                },
            };
            state.runtime.write().unwrap().lens = LensState {
                operation_id: Some(operation_id),
                stage: LensStage::Selecting,
                selection: Some(LensTargetSelection {
                    selection_id: operation_id,
                    stage: if published {
                        LensTargetSelectionStage::Reviewing
                    } else {
                        LensTargetSelectionStage::Picking
                    },
                    maximum_targets: MAX_LENS_TARGETS,
                    anchor: Some(window.facts.frame),
                    items: {
                        let mut old_window = window.clone();
                        old_window.identity.receipt = Uuid::from_u128(41);
                        let mut items = vec![LensTargetSelectionItem {
                            id: "old".into(),
                            window: old_window,
                            preview_uri: None,
                            preview_error: None,
                        }];
                        if published {
                            items.push(LensTargetSelectionItem {
                                id: "new".into(),
                                window,
                                preview_uri: None,
                                preview_error: None,
                            });
                        }
                        items
                    },
                    notice: Some("latest notice must survive rollback".into()),
                }),
                ..LensState::default()
            };
            let app = tauri::test::mock_builder()
                .manage(state)
                .build(crate::product_context())
                .unwrap();
            let handle = app.handle().clone();
            let mut pending = Box::pin(async move {
                let _addition = AddSelectionLease {
                    app: handle,
                    operation_id,
                    accepted_window: Some(Uuid::from_u128(42)),
                    preview_uri: None,
                };
                std::future::pending::<()>().await;
            });
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(std::future::Future::poll(pending.as_mut(), &mut context).is_pending());
            drop(pending);
            let selection = app.state::<AppState>().lens().unwrap().selection.unwrap();
            assert_eq!(selection.stage, LensTargetSelectionStage::Reviewing);
            assert_eq!(
                selection.notice.as_deref(),
                Some("latest notice must survive rollback")
            );
            assert_eq!(selection.items.len(), 1 + usize::from(published));
            assert_eq!(selection.items[0].id, "old");
            assert_eq!(
                *fixture.effects.lock().unwrap(),
                if published {
                    vec![]
                } else {
                    vec!["release_target"]
                }
            );
        }
    }

    #[test]
    fn agent_prompt_template_is_complete_bounded_and_has_deterministic_line_endings() {
        let normalized = AgentPromptTemplate {
            common: "First\r\n{turn_instruction}\rLast".into(),
            ..AgentPromptTemplate::default()
        }
        .normalize()
        .expect("valid template");
        assert_eq!(normalized.common, "First\n{turn_instruction}\nLast");

        let empty = AgentPromptTemplate {
            full_projection: " \n\t".into(),
            ..AgentPromptTemplate::default()
        };
        assert!(empty
            .normalize()
            .expect_err("empty section")
            .contains("must not be empty"));

        let maximum = AgentPromptTemplate {
            full_projection: "x".repeat(MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS),
            ..AgentPromptTemplate::default()
        };
        assert!(maximum.normalize().is_ok());

        let oversized = AgentPromptTemplate {
            full_projection: "x".repeat(MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS + 1),
            ..AgentPromptTemplate::default()
        };
        assert!(oversized
            .normalize()
            .expect_err("oversized section")
            .contains("must not exceed"));
    }

    #[test]
    fn failed_operation_preserves_identity_and_target() {
        let operation_id = Uuid::new_v4();
        let target = SelectedWindow {
            identity: crate::model::WindowIdentity {
                operation_id,
                receipt: Uuid::from_u128(42),
                selection_ordinal: 1,
            },
            facts: crate::model::WindowObservableFacts {
                application_id: "example.browser".into(),
                title: "Document".into(),
                application_name: "Browser".into(),
                frame: crate::model::Bounds {
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
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
    fn agent_retry_is_available_only_after_an_explicit_recoverable_stop() {
        assert!(can_retry_agent_transform(
            LensStage::AuthenticationRequired,
            true
        ));
        assert!(can_retry_agent_transform(LensStage::Failed, true));
        assert!(!can_retry_agent_transform(LensStage::Failed, false));
        assert!(!can_retry_agent_transform(LensStage::Ready, true));
        assert!(!can_retry_agent_transform(LensStage::Connecting, true));
        assert!(!can_retry_agent_transform(LensStage::Transforming, true));
        assert!(!can_retry_agent_transform(LensStage::Completed, true));
    }

    #[test]
    fn context_refresh_start_preserves_explicit_agent_recovery_state() {
        for stage in [
            LensStage::AuthenticationRequired,
            LensStage::Cancelled,
            LensStage::Failed,
        ] {
            let mut lens = LensState {
                stage,
                live: Some(LensLiveState {
                    lifecycle: LensMonitoringLifecycle::Watching,
                    health: LensSourceHealth::Healthy,
                    freshness: LensFreshness::Stale,
                    agent_refresh_interval_seconds: LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
                    last_outcome: Some(LensRefreshOutcome::Failed),
                    error: Some("recoverable failure".into()),
                }),
                error: Some("recoverable failure".into()),
                ..LensState::default()
            };

            mark_context_refresh_started(&mut lens);

            assert_eq!(lens.stage, stage);
            assert_eq!(lens.error.as_deref(), Some("recoverable failure"));
            let live = lens.live.as_ref().expect("live state");
            assert_eq!(live.freshness, LensFreshness::Stale);
            assert_eq!(live.last_outcome, Some(LensRefreshOutcome::Failed));
            assert_eq!(live.error.as_deref(), Some("recoverable failure"));
        }

        let mut settled = LensState {
            stage: LensStage::Completed,
            live: Some(LensLiveState {
                lifecycle: LensMonitoringLifecycle::Watching,
                health: LensSourceHealth::Healthy,
                freshness: LensFreshness::Current,
                agent_refresh_interval_seconds: LIVE_AGENT_REFRESH_INTERVAL_SECONDS,
                last_outcome: Some(LensRefreshOutcome::Unchanged),
                error: Some("obsolete diagnostic".into()),
            }),
            ..LensState::default()
        };

        mark_context_refresh_started(&mut settled);

        let live = settled.live.as_ref().expect("live state");
        assert_eq!(live.freshness, LensFreshness::Checking);
        assert_eq!(live.last_outcome, None);
        assert_eq!(live.error, None);
    }
}

#[tauri::command]
pub fn set_session_option<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    instance_id: Uuid,
    config_revision: u32,
    config_id: String,
    value: String,
) -> Result<(), String> {
    let controls = crate::session_controls::active_controls(&app, operation_id)?;
    controls.queue_change(instance_id, config_revision, config_id, value)?;
    controls.publish(&app)
}

#[tauri::command]
pub fn respond_agent_interaction<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation_id: Uuid,
    instance_id: Uuid,
    interaction_id: Uuid,
    response: crate::session_controls::InteractionResponse,
) -> Result<(), String> {
    let controls = crate::session_controls::active_controls(&app, operation_id)?;
    controls.respond(&app, instance_id, interaction_id, response)?;
    controls.publish(&app)
}

#[tauri::command]
pub async fn set_agent_defaults<R: tauri::Runtime>(
    app: AppHandle<R>,
    selection_id: Uuid,
    defaults: crate::agent_preferences::AgentDefaults,
    confirm_privilege: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let expected = state.snapshot()?;
    if expected.agent_selection.operation_id != Some(selection_id)
        || expected.agent_selection.selected_agent() != Some(expected.config.agent)
    {
        return Err("Agent selection changed".into());
    }
    let mode_id = expected
        .agent_selection
        .config_options
        .as_deref()
        .map(crate::session_controls::mode_option)
        .transpose()
        .map_err(|_| "Ambiguous Agent modes")?
        .flatten()
        .map(|o| o.id.to_string())
        .unwrap_or_else(|| "mode".into());
    let elevated = defaults.choices.iter().any(|c| {
        c.config_id == mode_id && Some(&c.value) != expected.agent_selection.policy_default.as_ref()
    });
    if elevated && !confirm_privilege {
        return Err("Confirm the shared mode policy before saving".into());
    }
    let options = agent::validate_agent_defaults(&app, &expected.config, &defaults).await?;
    // Cancel the old configuration's actor before changing persisted authority.
    state.agent_control.cancel_active()?;
    let snapshot = {
        let mut snapshot = state
            .runtime
            .write()
            .map_err(|_| "Application state is unavailable")?;
        if snapshot.config != expected.config
            || snapshot.agent_selection.operation_id != Some(selection_id)
        {
            return Err("Agent settings changed during validation".into());
        }
        let mut config = snapshot.config.clone();
        config.agent_preferences.set(config.agent, defaults);
        let revision = next_revision(&snapshot)?;
        state
            .store
            .save(&config)
            .map_err(|_| "Unable to save Agent defaults")?;
        snapshot.config = config;
        snapshot.agent_selection.config_options = options;
        snapshot.revision = revision;
        snapshot.clone()
    };
    emit_app_snapshot(&app, snapshot, true)?;
    Ok(())
}
