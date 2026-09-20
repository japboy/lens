//! History admission and display ownership, independent of live execution authority.
use crate::{
    app_state::{emit_app_snapshot, next_revision, AppState},
    model::{AgentKind, AgentSelectionStage, AgentSelectionState, LensStage, LensState},
    session_document::SessionDocument,
    session_history::{self, ProviderHistoryListing},
};
use agent_client_protocol::schema::v1::{ContentBlock, SessionUpdate};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use uuid::Uuid;

pub(crate) const RECENT_SESSION_COUNT: usize = 10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewPhase {
    #[default]
    Idle,
    Live,
    Loading,
    Ready,
    Failed,
}

pub(crate) mod wire;

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct SessionView {
    pub revision: u64,
    pub phase: ViewPhase,
    pub agent: Option<AgentKind>,
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub document: Option<SessionDocument>,
    pub error: Option<String>,
    #[serde(skip)]
    generation: Uuid,
    #[serde(skip)]
    operation_id: Option<Uuid>,
    #[serde(skip)]
    entry_revisions: Vec<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct HistoryEntry {
    pub agent: AgentKind,
    pub session_id: String,
    pub cwd: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub can_load: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HistoryCatalog {
    pub cwd: PathBuf,
    pub generation: Uuid,
    pub loading: bool,
    pub entries: Vec<HistoryEntry>,
    pub notices: Vec<String>,
}

#[derive(Default)]
pub(crate) struct SessionViewStore {
    /// Lock order: admission -> app runtime -> view -> display. Never held over an await.
    pub admission: Mutex<()>,
    inner: Mutex<SessionView>,
    catalog: Mutex<HistoryCatalog>,
    display: Mutex<DisplayNotifications>,
}

/// Constant-space display queue. Canonical updates are never queued or discarded here.
#[derive(Default)]
struct DisplayNotifications {
    scheduled: bool,
    pending: Option<DisplayChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisplayChange {
    generation: Uuid,
    base_revision: u64,
    index: Option<usize>,
}

impl DisplayNotifications {
    /// Returns true only for the caller that must install a flush task.
    fn push(&mut self, change: DisplayChange) -> bool {
        match self.pending.as_mut() {
            Some(pending) if pending.generation == change.generation => {
                if pending.index != change.index {
                    pending.index = None;
                }
            }
            _ => self.pending = Some(change),
        }
        if self.scheduled {
            false
        } else {
            self.scheduled = true;
            true
        }
    }

    /// Immediate lifecycle events supersede pending display work, not its timer.
    fn supersede(&mut self) {
        self.pending = None;
    }

    fn take(&mut self, generation: Uuid) -> Option<DisplayChange> {
        self.scheduled = false;
        self.pending
            .take()
            .filter(|pending| pending.generation == generation)
    }
}

fn lock_error<T>(_: T) -> String {
    "Session view state is unavailable".into()
}

impl SessionViewStore {
    pub fn phase(&self) -> Result<ViewPhase, String> {
        self.inner.lock().map(|view| view.phase).map_err(lock_error)
    }
    #[cfg(test)]
    pub fn view(&self) -> Result<SessionView, String> {
        self.inner
            .lock()
            .map(|view| view.clone())
            .map_err(lock_error)
    }
    pub fn catalog(&self) -> Result<HistoryCatalog, String> {
        self.catalog
            .lock()
            .map(|catalog| catalog.clone())
            .map_err(lock_error)
    }
    pub fn ensure_not_loading(&self) -> Result<(), String> {
        if self.phase()? == ViewPhase::Loading {
            return Err("Wait for the session history to finish loading or close it".into());
        }
        Ok(())
    }
    pub fn clear(&self) -> Result<(), String> {
        let mut view = self.inner.lock().map_err(lock_error)?;
        *view = SessionView {
            revision: view.revision + 1,
            ..Default::default()
        };
        Ok(())
    }
}

pub(crate) fn active_session(lens: &LensState) -> bool {
    lens.live.is_some()
        || lens.selection.is_some()
        || lens.input.is_some()
        || lens.target_set.is_some()
        || matches!(
            lens.stage,
            LensStage::Selecting
                | LensStage::Extracting
                | LensStage::Ready
                | LensStage::Connecting
                | LensStage::AuthenticationRequired
                | LensStage::Transforming
        )
}

fn selection_busy(stage: AgentSelectionStage) -> bool {
    matches!(
        stage,
        AgentSelectionStage::Checking
            | AgentSelectionStage::Authenticating
            | AgentSelectionStage::SigningOut
    )
}

pub(crate) fn history_enabled<R: Runtime>(app: &AppHandle<R>) -> Result<bool, String> {
    let state = app.state::<AppState>();
    let snapshot = state.snapshot()?;
    Ok(!active_session(&snapshot.lens)
        && !selection_busy(snapshot.agent_selection.stage)
        && state.session_view.phase()? != ViewPhase::Loading)
}

pub(crate) fn emit<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let state = app.state::<AppState>();
    // Keep lifecycle publication ordered with mutations and timer flushes.
    let view = state.session_view.inner.lock().map_err(lock_error)?;
    let mut display = state.session_view.display.lock().map_err(lock_error)?;
    display.supersede();
    app.emit_to(
        crate::ui::LENS_WINDOW_LABEL,
        "session-view-changed",
        view.wire(None),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn get_session_view<R: Runtime>(
    webview: tauri::Webview<R>,
    state: tauri::State<'_, AppState>,
) -> Result<wire::WireView, String> {
    if webview.label() != crate::ui::LENS_WINDOW_LABEL {
        return Err("Session content is only available to the Lens overlay".into());
    }
    Ok(state
        .session_view
        .inner
        .lock()
        .map_err(lock_error)?
        .wire(None))
}

#[tauri::command]
pub(crate) fn close_session_view<R: Runtime>(
    webview: tauri::Webview<R>,
    app: AppHandle<R>,
) -> Result<(), String> {
    if webview.label() != crate::ui::LENS_WINDOW_LABEL {
        return Err("Session content is only available to the Lens overlay".into());
    }
    let state = app.state::<AppState>();
    let _guard = state.session_view.admission.lock().map_err(lock_error)?;
    if active_session(&state.lens()?) {
        return Err("Close the active Lens session through its live controls".into());
    }
    state.session_view.clear()?;
    emit(&app)?;
    crate::ui::sync_history_menu(&app)
}

pub(crate) fn start_live<R: Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    agent: AgentKind,
    session_id: String,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snapshot = state.runtime.read().map_err(lock_error)?;
    if snapshot.lens.operation_id != Some(operation_id) || !active_session(&snapshot.lens) {
        return Ok(());
    }
    let mut view = state.session_view.inner.lock().map_err(lock_error)?;
    *view = SessionView {
        revision: view.revision + 1,
        phase: ViewPhase::Live,
        agent: Some(agent),
        session_id: Some(session_id),
        document: Some(SessionDocument::default()),
        operation_id: Some(operation_id),
        generation: Uuid::new_v4(),
        ..Default::default()
    };
    drop(view);
    drop(snapshot);
    emit(app)
}

fn mutate_live<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    update: impl FnOnce(&mut SessionDocument) -> Result<(), String>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snapshot = state.runtime.read().map_err(lock_error)?;
    let mut view = state.session_view.inner.lock().map_err(lock_error)?;
    if view.phase != ViewPhase::Live
        || view.session_id.as_deref() != Some(session_id)
        || snapshot.lens.operation_id != view.operation_id
        || !active_session(&snapshot.lens)
    {
        return Ok(());
    }
    let mut failed = false;
    let changed = if let Some(document) = view.document.as_mut() {
        match update(document) {
            Ok(()) => document.last_changed_entry(),
            Err(error) => {
                view.error = Some(error);
                failed = true;
                None
            }
        }
    } else {
        None
    };
    if changed.is_none() && !failed {
        return Ok(());
    }
    let base_revision = view.revision;
    view.revision += 1;
    let revision = view.revision;
    if let Some(index) = changed {
        if view.entry_revisions.len() <= index {
            view.entry_revisions.resize(index + 1, base_revision);
        }
        view.entry_revisions[index] = revision;
    }
    let schedule = state
        .session_view
        .display
        .lock()
        .map_err(lock_error)?
        .push(DisplayChange {
            generation: view.generation,
            base_revision,
            index: changed,
        });
    drop(view);
    drop(snapshot);
    if schedule {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(16)).await;
            if let Err(error) = flush_display(&app) {
                eprintln!("Unable to publish session display update: {error}");
            }
        });
    }
    Ok(())
}

fn flush_display<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let view = state.session_view.inner.lock().map_err(lock_error)?;
    let mut display = state.session_view.display.lock().map_err(lock_error)?;
    let Some(change) = display.take(view.generation) else {
        return Ok(());
    };
    // Only the final manifest/entry descriptor is built; no history body is cloned.
    let wire = view.wire(change.index.map(|index| (change.base_revision, index)));
    app.emit_to(crate::ui::LENS_WINDOW_LABEL, "session-view-changed", wire)
        .map_err(|error| error.to_string())
}

pub(crate) fn append_prompt<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    prompt: &[ContentBlock],
) -> Result<(), String> {
    mutate_live(app, session_id, |document| document.append_prompt(prompt))
}
pub(crate) fn record_live<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    update: SessionUpdate,
) -> Result<(), String> {
    mutate_live(app, session_id, |document| document.record_update(update))
}

pub(crate) fn agent_label(agent: AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "Claude",
        AgentKind::Codex => "Codex",
        AgentKind::External(_) => "External ACP",
    }
}

fn timestamp(value: &Option<String>) -> Option<i64> {
    value
        .as_ref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|date| date.timestamp_millis())
}

fn merge_listings(
    cwd: &Path,
    listings: Vec<(AgentKind, Result<ProviderHistoryListing, String>)>,
) -> HistoryCatalog {
    let mut catalog = HistoryCatalog {
        cwd: cwd.to_path_buf(),
        generation: Uuid::new_v4(),
        ..Default::default()
    };
    let mut seen = std::collections::HashSet::new();
    for (agent, result) in listings {
        match result {
            Ok(listing) => {
                if let Some(error) = listing.error {
                    catalog
                        .notices
                        .push(format!("{}: {error}", agent_label(agent)));
                }
                if !listing.complete {
                    catalog
                        .notices
                        .push(format!("{}: partial history", agent_label(agent)));
                }
                for entry in listing.entries {
                    if Path::new(&entry.cwd) != cwd {
                        continue;
                    }
                    if !seen.insert((agent, entry.session_id.clone())) {
                        continue;
                    }
                    catalog.entries.push(HistoryEntry {
                        agent,
                        session_id: entry.session_id,
                        cwd: entry.cwd,
                        title: entry
                            .title
                            .filter(|title| !title.trim().is_empty())
                            .unwrap_or_else(|| "Untitled session".into()),
                        updated_at: entry.updated_at,
                        can_load: listing.can_load,
                    });
                }
            }
            Err(error) => catalog
                .notices
                .push(format!("{}: {error}", agent_label(agent))),
        }
    }
    let undated = catalog
        .entries
        .iter()
        .filter(|entry| timestamp(&entry.updated_at).is_none())
        .count();
    if undated > 0 {
        catalog
            .notices
            .push(format!("{undated} sessions have no valid update time"));
    }
    catalog
        .entries
        .retain(|entry| timestamp(&entry.updated_at).is_some());
    catalog.entries.sort_by(|a, b| {
        timestamp(&b.updated_at)
            .cmp(&timestamp(&a.updated_at))
            .then_with(|| a.agent.cmp(&b.agent))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    catalog.entries.truncate(RECENT_SESSION_COUNT);
    catalog
}

/// Caller holds admission while changing configuration and invalidating its history scope.
pub(crate) fn invalidate_working_directory<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let cwd = crate::store::effective_working_directory(&state.config()?);
    *state.session_view.catalog.lock().map_err(lock_error)? = HistoryCatalog {
        cwd,
        generation: Uuid::new_v4(),
        ..Default::default()
    };
    let mut view = state.session_view.inner.lock().map_err(lock_error)?;
    if matches!(
        view.phase,
        ViewPhase::Loading | ViewPhase::Ready | ViewPhase::Failed
    ) {
        *view = SessionView {
            revision: view.revision + 1,
            ..Default::default()
        };
    }
    drop(view);
    emit(app)?;
    crate::ui::sync_history_menu(app)
}

// External catalog membership is not execution authority. Only a successful
// selection in this process admits automatic external history discovery.
fn history_agents(selection: &crate::model::AgentSelectionState) -> Vec<AgentKind> {
    let mut kinds = vec![AgentKind::Claude, AgentKind::Codex];
    if let Some(kind @ AgentKind::External(_)) = selection.selected_agent() {
        kinds.push(kind);
    }
    kinds
}

pub(crate) async fn refresh<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let (generation, cwd) = {
        let state = app.state::<AppState>();
        let _guard = state.session_view.admission.lock().map_err(lock_error)?;
        let cwd = crate::store::effective_working_directory(&state.config()?);
        let mut catalog = state.session_view.catalog.lock().map_err(lock_error)?;
        if catalog.loading && catalog.cwd == cwd {
            return Ok(());
        }
        if catalog.cwd != cwd {
            *catalog = HistoryCatalog {
                cwd: cwd.clone(),
                ..Default::default()
            };
        }
        catalog.loading = true;
        catalog.generation = Uuid::new_v4();
        (catalog.generation, cwd)
    };
    if let Err(error) = crate::ui::sync_history_menu(&app) {
        let state = app.state::<AppState>();
        let mut catalog = state.session_view.catalog.lock().map_err(lock_error)?;
        if catalog.generation == generation {
            catalog.loading = false;
        }
        return Err(error);
    }
    let snapshot = app.state::<AppState>().snapshot()?;
    let kinds = history_agents(&snapshot.agent_selection);
    let mut pending = tokio::task::JoinSet::new();
    for kind in kinds {
        let app = app.clone();
        let cwd = cwd.clone();
        pending
            .spawn(async move { (kind, session_history::list_provider(&app, kind, &cwd).await) });
    }
    let mut listings = Vec::new();
    while let Some(result) = pending.join_next().await {
        listings.push(result.map_err(|_| "Agent history task failed")?);
    }
    let mut merged = merge_listings(&cwd, listings);
    merged.generation = generation;
    commit_catalog(&app, merged)?;
    crate::ui::sync_history_menu(&app)
}

fn commit_catalog<R: Runtime>(app: &AppHandle<R>, merged: HistoryCatalog) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _guard = state.session_view.admission.lock().map_err(lock_error)?;
    let current_cwd = crate::store::effective_working_directory(&state.config()?);
    let mut catalog = state.session_view.catalog.lock().map_err(lock_error)?;
    if catalog.generation == merged.generation && current_cwd == merged.cwd {
        *catalog = merged;
    }
    Ok(())
}

pub(crate) async fn open<R: Runtime>(
    app: AppHandle<R>,
    catalog_generation: Uuid,
    index: usize,
) -> Result<(), String> {
    let (entry, generation, config) = {
        let state = app.state::<AppState>();
        let _guard = state.session_view.admission.lock().map_err(lock_error)?;
        if !history_enabled(&app)? {
            return Err("End the active session before opening history".into());
        }
        let catalog = state.session_view.catalog()?;
        if catalog.generation != catalog_generation {
            return Err("Session history changed; reopen the menu".into());
        }
        let entry = catalog
            .entries
            .get(index)
            .cloned()
            .ok_or("Session history entry is unavailable")?;
        if !entry.can_load {
            return Err("This Agent cannot load session history".into());
        }
        let config = state.config()?;
        let effective_cwd = crate::store::effective_working_directory(&config);
        if catalog.cwd != effective_cwd || Path::new(&entry.cwd) != effective_cwd {
            return Err("Working Directory changed; refresh session history".into());
        }
        let generation = Uuid::new_v4();
        let mut view = state.session_view.inner.lock().map_err(lock_error)?;
        *view = SessionView {
            revision: view.revision + 1,
            phase: ViewPhase::Loading,
            agent: Some(entry.agent),
            session_id: Some(entry.session_id.clone()),
            title: Some(entry.title.clone()),
            generation,
            ..Default::default()
        };
        (entry, generation, config)
    };
    if let Err(error) = crate::ui::show_history_window(&app) {
        fail(&app, generation, error.to_string())?;
        return Err(error.to_string());
    }
    if let Err(error) = emit(&app).and_then(|()| crate::ui::sync_history_menu(&app)) {
        fail(&app, generation, error.clone())?;
        return Err(error);
    }
    let loaded =
        session_history::load_provider(&app, entry.agent, &entry.session_id, &entry.cwd).await;
    let result = (|| {
        let state = app.state::<AppState>();
        let _guard = state.session_view.admission.lock().map_err(lock_error)?;
        let mut snapshot = state.runtime.write().map_err(lock_error)?;
        let mut view = state.session_view.inner.lock().map_err(lock_error)?;
        if view.generation != generation || view.phase != ViewPhase::Loading {
            return Ok(None);
        }
        if active_session(&snapshot.lens)
            || !snapshot.config.same_execution_config(&config)
            || crate::store::effective_working_directory(&snapshot.config) != entry.cwd
        {
            return Err("Session loading was superseded".to_string());
        }
        let document = loaded?;
        let mut selected = snapshot.config.clone();
        selected.agent = entry.agent;
        let revision = next_revision(&snapshot)?;
        state
            .store
            .save(&selected)
            .map_err(|error| error.to_string())?;
        snapshot.config = selected;
        snapshot.agent_selection = selection_after_history(&snapshot.agent_selection, entry.agent);
        snapshot.revision = revision;
        view.document = Some(document);
        view.phase = ViewPhase::Ready;
        view.revision += 1;
        Ok(Some(snapshot.clone()))
    })();
    match result {
        Ok(Some(snapshot)) => {
            emit_app_snapshot(&app, snapshot, true)?;
            emit(&app)?;
        }
        Ok(None) => {}
        Err(error) => {
            fail(&app, generation, error.clone())?;
            return Err(error);
        }
    }
    crate::ui::sync_history_menu(&app)
}

fn selection_after_history(current: &AgentSelectionState, agent: AgentKind) -> AgentSelectionState {
    if current.selected_agent() == Some(agent) {
        return current.clone();
    }
    AgentSelectionState {
        candidate: Some(agent),
        stage: AgentSelectionStage::HistorySelected,
        operation_id: Some(Uuid::new_v4()),
        message: Some(format!(
            "{} chosen from session history. Select this Agent to verify live readiness.",
            agent_label(agent)
        )),
        ..Default::default()
    }
}

fn fail<R: Runtime>(app: &AppHandle<R>, generation: Uuid, error: String) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut view = state.session_view.inner.lock().map_err(lock_error)?;
        if view.generation != generation {
            return Ok(());
        }
        view.phase = ViewPhase::Failed;
        view.error = Some(error);
        view.revision += 1;
    }
    emit(app)?;
    crate::ui::sync_history_menu(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn burst_keeps_one_timer_and_first_patch_base() {
        let generation = Uuid::new_v4();
        let mut queue = DisplayNotifications::default();
        for revision in 10..10_000 {
            assert_eq!(
                queue.push(DisplayChange {
                    generation,
                    base_revision: revision,
                    index: Some(2)
                }),
                revision == 10
            );
        }
        assert_eq!(
            queue.take(generation),
            Some(DisplayChange {
                generation,
                base_revision: 10,
                index: Some(2)
            })
        );
        assert!(!queue.scheduled);
        assert!(queue.pending.is_none());
    }

    #[test]
    fn different_entries_and_errors_require_full_manifest() {
        let generation = Uuid::new_v4();
        for next in [Some(3), None] {
            let mut queue = DisplayNotifications::default();
            queue.push(DisplayChange {
                generation,
                base_revision: 1,
                index: Some(2),
            });
            queue.push(DisplayChange {
                generation,
                base_revision: 2,
                index: next,
            });
            queue.push(DisplayChange {
                generation,
                base_revision: 3,
                index: Some(2),
            });
            assert_eq!(
                queue.take(generation),
                Some(DisplayChange {
                    generation,
                    base_revision: 1,
                    index: None
                })
            );
        }
    }

    #[test]
    fn lifecycle_supersedes_old_work_without_installing_another_timer() {
        let old = Uuid::new_v4();
        let new = Uuid::new_v4();
        let mut queue = DisplayNotifications::default();
        assert!(queue.push(DisplayChange {
            generation: old,
            base_revision: 1,
            index: Some(0)
        }));
        queue.supersede();
        assert!(!queue.push(DisplayChange {
            generation: new,
            base_revision: 3,
            index: Some(0)
        }));
        assert_eq!(
            queue.take(new),
            Some(DisplayChange {
                generation: new,
                base_revision: 3,
                index: Some(0)
            })
        );
        queue.push(DisplayChange {
            generation: old,
            base_revision: 1,
            index: Some(0),
        });
        assert_eq!(queue.take(new), None);
        assert!(!queue.scheduled);
        assert!(queue.pending.is_none());
    }

    #[test]
    fn completed_monitoring_remains_active_and_history_is_not_live() {
        let lens = LensState {
            stage: LensStage::Completed,
            ..Default::default()
        };
        assert!(!active_session(&lens));
        assert!(active_session(&LensState {
            stage: LensStage::Selecting,
            ..Default::default()
        }));
        assert!(active_session(&LensState {
            stage: LensStage::AuthenticationRequired,
            ..Default::default()
        }));
    }
    #[test]
    fn chronology_parses_offsets_instead_of_lexical_order() {
        assert_eq!(
            timestamp(&Some("2026-09-17T09:00:00+09:00".into())),
            timestamp(&Some("2026-09-17T00:00:00Z".into()))
        );
        assert_eq!(timestamp(&Some("invalid".into())), None);
    }
}

#[cfg(test)]
#[path = "session_view_tests.rs"]
mod integration_tests;
