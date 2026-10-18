//! Whole-cache history management. Remote deletion is confirmed before cache removal.
use crate::{
    app_state::AppState,
    history_catalog,
    model::{AgentKind, AppConfig},
    session_history,
    session_history_store::{HistoryRecord, HistoryScope, HistorySource, ScanOutcome},
    session_view,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Serialize)]
pub(crate) struct HistoryPage {
    pub revision: i64,
    pub total: usize,
    pub entries: Vec<HistoryRow>,
}
#[derive(Serialize)]
pub(crate) struct HistoryRow {
    pub id: String,
    pub title: String,
    pub agent: String,
    pub cwd: String,
    pub updated_at: Option<String>,
    pub state: &'static str,
    pub available: bool,
}
#[derive(Debug, Default, Serialize)]
pub(crate) struct HistoryResult {
    pub succeeded: usize,
    pub failed: usize,
    pub purged: usize,
    pub errors: Vec<String>,
}

/// Reset even when a command is cancelled; no standard mutex survives an await.
struct ManagementLease<'a>(&'a AtomicBool);
impl Drop for ManagementLease<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn admit<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if !session_view::history_enabled(app)? {
        return Err("End the active session before managing history".into());
    }
    Ok(())
}
fn identity(record: &HistoryRecord) -> Result<String, String> {
    serde_json::to_string(&(
        record.source.agent,
        &record.source.invocation,
        &record.entry.cwd,
        &record.entry.session_id,
    ))
    .map_err(|e| e.to_string())
}
fn reconstructable(config: &AppConfig, source: &HistorySource) -> bool {
    history_catalog::source(config, source.agent).is_ok_and(|current| current == *source)
}
fn scope_label(config: &AppConfig, source: &HistorySource, cwd: &str) -> String {
    let agent = match source.agent {
        AgentKind::External(id) => config
            .external_agents
            .iter()
            .find(|profile| profile.id == id && reconstructable(config, source))
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| format!("External ACP {id} ({})", source.invocation)),
        agent => session_view::agent_label(agent).into(),
    };
    format!("{agent} · {cwd}")
}

fn unchanged<R: Runtime>(
    app: &AppHandle<R>,
    config: &AppConfig,
    source: &HistorySource,
) -> Result<(), String> {
    let current = app.state::<AppState>().config()?;
    if !current.same_execution_config(config) || !history_catalog::same_source(&current, source) {
        return Err("Agent configuration changed during history management".into());
    }
    Ok(())
}
async fn flush<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let handle = app.clone();
    tokio::task::spawn_blocking(move || {
        history_catalog::writer(&handle.state::<AppState>())?.flush()
    })
    .await
    .map_err(|_| "History storage task failed".to_string())?
}
fn check_revision(
    store: &crate::session_history_store::HistoryStore,
    revision: i64,
) -> Result<(), String> {
    if store.current_revision()? != revision {
        return Err("Session history changed; reload the list".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn list_history<R: Runtime>(
    app: AppHandle<R>,
    offset: usize,
    limit: usize,
) -> Result<HistoryPage, String> {
    flush(&app).await?;
    let state = app.state::<AppState>();
    let config = state.config()?;
    let store = history_catalog::store(&state)?;
    let page = tokio::task::spawn_blocking(move || store.page(offset, limit.min(100)))
        .await
        .map_err(|_| "History storage task failed")??;
    let entries = page
        .entries
        .into_iter()
        .map(|record| {
            let id = identity(&record)?;
            let available = reconstructable(&config, &record.source);
            let agent = match record.source.agent {
                AgentKind::External(id) => config
                    .external_agents
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Unavailable external Agent".into()),
                agent => session_view::agent_label(agent).into(),
            };
            let entry = record.entry;
            let valid = |value: Option<String>| {
                value.filter(|v| chrono::DateTime::parse_from_rfc3339(v).is_ok())
            };
            let updated_at = match (
                valid(entry.provider_updated_at),
                valid(entry.local_activity_at),
            ) {
                (Some(a), Some(b)) => {
                    let date = |v: &str| chrono::DateTime::parse_from_rfc3339(v).ok();
                    Some(if date(&a) >= date(&b) { a } else { b })
                }
                (a, b) => a.or(b),
            };
            Ok(HistoryRow {
                id,
                title: entry.title.unwrap_or_else(|| "Untitled session".into()),
                agent,
                cwd: entry.cwd,
                updated_at,
                available,
                state: match entry.state {
                    crate::session_history_store::EntryState::Listed => "listed",
                    crate::session_history_store::EntryState::LocalOnly => "local_only",
                    crate::session_history_store::EntryState::NotSeen => "not_seen",
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(HistoryPage {
        revision: page.revision,
        total: page.total,
        entries,
    })
}

async fn storage<T: Send + 'static>(
    store: &std::sync::Arc<crate::session_history_store::HistoryStore>,
    work: impl FnOnce(&crate::session_history_store::HistoryStore) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || work(&store))
        .await
        .map_err(|_| "History storage task failed".to_string())?
}

fn begin_management<R: Runtime>(app: &AppHandle<R>) -> Result<AppConfig, String> {
    let state = app.state::<AppState>();
    let _admission = state
        .session_view
        .admission
        .lock()
        .map_err(|_| "History admission lock poisoned")?;
    admit(app)?;
    let snapshot = state
        .runtime
        .write()
        .map_err(|_| "Application state lock poisoned")?;
    if session_view::active_session(&snapshot.lens) {
        return Err("End the active session before managing history".into());
    }
    state
        .session_view
        .management_active
        .store(true, Ordering::Release);
    Ok(snapshot.config.clone())
}

fn clear_history_view<R: Runtime>(
    app: &AppHandle<R>,
    source: &HistorySource,
    cwd: &str,
    ids: Option<&[String]>,
) -> Result<(), String> {
    if app
        .state::<AppState>()
        .session_view
        .clear_history(source, cwd, ids)?
    {
        session_view::emit(app)?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn sync_history<R: Runtime>(app: AppHandle<R>) -> Result<HistoryResult, String> {
    let state = app.state::<AppState>();
    let _lane = state.session_view.history_gate.lock().await;
    let config = begin_management(&app)?;
    let _lease = ManagementLease(&state.session_view.management_active);
    flush(&app).await?;
    let store = history_catalog::store(&state)?;
    let scopes = storage(&store, |store| store.scopes()).await?;
    let mut result = HistoryResult::default();
    for scope in scopes {
        let label = scope_label(&config, &scope.source, &scope.cwd);
        let source = scope.source.clone();
        let cwd = scope.cwd.clone();
        let token = match storage(&store, move |store| store.begin_scan(&source, &cwd)).await {
            Ok(token) => token,
            Err(error) => {
                result.failed += 1;
                result.errors.push(format!("{label}: {error}"));
                continue;
            }
        };
        if !reconstructable(&config, &scope.source) {
            let handle = app.clone();
            let expected = history_catalog::sources(&config)?;
            let removed = storage(&store, move |store| {
                let state = handle.state::<AppState>();
                let _admission = state
                    .session_view
                    .admission
                    .lock()
                    .map_err(|_| "History admission lock poisoned")?;
                if history_catalog::sources(&state.config()?)? != expected {
                    return Err("Settings changed during history synchronization".into());
                }
                store.prune_scope(&token)
            })
            .await;
            match removed {
                Ok(count) => {
                    result.purged += count;
                    if let Err(error) = clear_history_view(&app, &scope.source, &scope.cwd, None) {
                        result
                            .errors
                            .push(format!("{label}: History display update failed: {error}"));
                    }
                }
                Err(error) => {
                    result.failed += 1;
                    result.errors.push(format!("{label}: {error}"));
                }
            }
            continue;
        }
        let listing =
            session_history::list_provider(&app, scope.source.agent, &config, &scope.cwd).await;
        if let Err(error) = unchanged(&app, &config, &scope.source) {
            result.failed += 1;
            result.errors.push(format!("{label}: {error}"));
            break;
        }
        let committed = storage(&store, move |store| match listing {
            Ok(listing) => {
                history_catalog::apply_listing(store, &token, &listing)?;
                store.finish_scan(
                    &token,
                    &history_catalog::now(),
                    if listing.complete {
                        ScanOutcome::Complete
                    } else {
                        ScanOutcome::Partial
                    },
                    listing.error.as_deref(),
                )?;
                Ok((listing.complete, listing.error))
            }
            Err(error) => {
                store.finish_scan(
                    &token,
                    &history_catalog::now(),
                    ScanOutcome::Failed,
                    Some(&error),
                )?;
                Ok((false, Some(error)))
            }
        })
        .await;
        match committed {
            Ok((true, _)) => result.succeeded += 1,
            Ok((false, error)) => {
                result.failed += 1;
                result.errors.push(format!(
                    "{}: {}",
                    label,
                    error.unwrap_or_else(|| "Incomplete session list".into())
                ));
            }
            Err(error) => {
                result.failed += 1;
                result.errors.push(format!("{label}: {error}"));
            }
        }
    }
    if let Err(error) = session_view::refresh(app.clone()).await {
        result
            .errors
            .push(format!("History menu refresh failed: {error}"));
    }
    Ok(result)
}

#[tauri::command]
pub(crate) async fn delete_history<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    revision: i64,
) -> Result<HistoryResult, String> {
    delete(app, Some(id), revision).await
}
#[tauri::command]
pub(crate) async fn delete_all_history<R: Runtime>(
    app: AppHandle<R>,
    revision: i64,
) -> Result<HistoryResult, String> {
    delete(app, None, revision).await
}
fn confirm_deletions(
    store: &crate::session_history_store::HistoryStore,
    token: &crate::session_history_store::ScanToken,
    deleted: Vec<(String, Result<(), String>)>,
    result: &mut HistoryResult,
) -> Result<(), String> {
    let mut confirmed = Vec::new();
    for (id, status) in deleted {
        match status {
            Ok(()) => confirmed.push(id),
            Err(error) => {
                result.failed += 1;
                result.errors.push(format!("{id}: {error}"));
            }
        }
    }
    let removed = store.remove_confirmed_batch(token, &confirmed)?;
    result.succeeded += removed;
    if removed != confirmed.len() {
        result.failed += confirmed.len() - removed;
        result.errors.push(
            "Agent deletion succeeded but cache changed; reload the list and retry deletion".into(),
        );
    }
    Ok(())
}

async fn delete<R: Runtime>(
    app: AppHandle<R>,
    id: Option<String>,
    revision: i64,
) -> Result<HistoryResult, String> {
    let state = app.state::<AppState>();
    let _lane = state.session_view.history_gate.lock().await;
    let config = begin_management(&app)?;
    let _lease = ManagementLease(&state.session_view.management_active);
    flush(&app).await?;
    let store = history_catalog::store(&state)?;
    let records = storage(&store, move |store| {
        check_revision(store, revision)?;
        store.records()
    })
    .await?
    .into_iter()
    .filter(|record| {
        id.as_ref()
            .is_none_or(|id| identity(record).as_ref() == Ok(id))
    })
    .collect::<Vec<_>>();
    if id.is_some() && records.is_empty() {
        return Err("Session history entry is unavailable".into());
    }
    let mut scopes = Vec::<HistoryScope>::new();
    for record in &records {
        if !scopes
            .iter()
            .any(|s| s.source == record.source && s.cwd == record.entry.cwd)
        {
            scopes.push(HistoryScope {
                source: record.source.clone(),
                cwd: record.entry.cwd.clone(),
            });
        }
    }
    let mut result = HistoryResult::default();
    for scope in scopes {
        let label = scope_label(&config, &scope.source, &scope.cwd);
        let ids = records
            .iter()
            .filter(|r| r.source == scope.source && r.entry.cwd == scope.cwd)
            .map(|r| r.entry.session_id.clone())
            .collect::<Vec<_>>();
        if !reconstructable(&config, &scope.source) {
            result.failed += ids.len();
            result.errors.push(format!("{label}: Agent definition is unavailable; synchronize history to remove obsolete cache entries"));
            continue;
        }
        let source = scope.source.clone();
        let cwd = scope.cwd.clone();
        let token = match storage(&store, move |store| store.begin_scan(&source, &cwd)).await {
            Ok(token) => token,
            Err(error) => {
                result.failed += ids.len();
                result.errors.push(format!("{label}: {error}"));
                continue;
            }
        };
        match session_history::delete_provider(
            &app,
            scope.source.agent,
            &config,
            &scope.cwd,
            ids.clone(),
        )
        .await
        {
            Ok(deleted) => {
                let confirmed = deleted
                    .iter()
                    .filter(|(_, status)| status.is_ok())
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                let count = ids.len();
                match storage(&store, move |store| {
                    let mut outcome = HistoryResult::default();
                    confirm_deletions(store, &token, deleted, &mut outcome)?;
                    Ok(outcome)
                })
                .await
                {
                    Ok(outcome) => {
                        result.succeeded += outcome.succeeded;
                        result.failed += outcome.failed;
                        result.errors.extend(
                            outcome
                                .errors
                                .into_iter()
                                .map(|error| format!("{label}: {error}")),
                        );
                    }
                    Err(error) => {
                        result.failed += count;
                        result.errors.push(format!(
                            "{label}: Agent responded but cache cleanup failed: {error}"
                        ));
                    }
                }
                if let Err(error) =
                    clear_history_view(&app, &scope.source, &scope.cwd, Some(&confirmed))
                {
                    result
                        .errors
                        .push(format!("{label}: History display update failed: {error}"));
                }
                if let Err(error) = unchanged(&app, &config, &scope.source) {
                    result.errors.push(format!("{label}: {error}"));
                    break;
                }
            }
            Err(error) => {
                result.failed += ids.len();
                result.errors.push(format!("{label}: {error}"));
            }
        }
    }
    if let Err(error) = session_view::refresh(app.clone()).await {
        result
            .errors
            .push(format!("History menu refresh failed: {error}"));
    }
    Ok(result)
}

#[tauri::command]
pub(crate) async fn open_history<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    revision: i64,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _lane = state.session_view.history_gate.lock().await;
    flush(&app).await?;
    let store = history_catalog::store(&state)?;
    let record = storage(&store, move |store| {
        check_revision(store, revision)?;
        store
            .records()?
            .into_iter()
            .find(|record| identity(record).as_ref() == Ok(&id))
            .ok_or_else(|| "Session history entry is unavailable".to_string())
    })
    .await?;
    session_view::open_record(
        app.clone(),
        session_view::HistoryEntry {
            agent: record.source.agent,
            invocation: record.source.invocation,
            session_id: record.entry.session_id,
            cwd: record.entry.cwd,
            title: record
                .entry
                .title
                .unwrap_or_else(|| "Untitled session".into()),
            updated_at: record.entry.provider_updated_at,
            presence: record.entry.state,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::{AgentDescriptor, AgentHost, HostFuture},
        agent_environment::EnvironmentPurpose,
        agent_runtime::ResolvedAgentRuntime,
        session_history_store::{EntryState, StoredEntry},
    };
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
    };
    use tauri::test::MockRuntime;
    struct FixtureHost {
        visits: Arc<Mutex<Vec<String>>>,
    }
    impl AgentHost<MockRuntime> for FixtureHost {
        fn resolve<'a>(
            &'a self,
            _: &'a AppHandle<MockRuntime>,
            _: AgentKind,
        ) -> HostFuture<'a, ResolvedAgentRuntime> {
            panic!("must not install")
        }
        fn resolve_installed<'a>(
            &'a self,
            _: &'a AppHandle<MockRuntime>,
            kind: AgentKind,
        ) -> HostFuture<'a, Option<ResolvedAgentRuntime>> {
            Box::pin(async move {
                Ok(Some(ResolvedAgentRuntime {
                    kind,
                    adapter_name: "fixture",
                    adapter_version: "1".into(),
                    command: "/must-not-spawn".into(),
                    args: vec![],
                    installation: None,
                }))
            })
        }
        fn connect(
            &self,
            _: &AgentDescriptor,
            cwd: PathBuf,
            _: EnvironmentPurpose,
        ) -> agent_client_protocol::DynConnectTo<agent_client_protocol::Client> {
            use agent_client_protocol::{
                schema::v1::*, Agent, Client, ConnectionTo, DynConnectTo, Responder,
            };
            self.visits
                .lock()
                .unwrap()
                .push(cwd.to_string_lossy().into());
            let failure = cwd == std::path::Path::new("/offline");
            let agent = Agent.builder().on_receive_request(
                async move |_: InitializeRequest, responder: Responder<InitializeResponse>, _: ConnectionTo<Client>| {
                    if failure { return responder.respond_with_error(agent_client_protocol::Error::internal_error().data("Synthetic offline")); }
                    responder.respond(serde_json::from_value(serde_json::json!({"protocolVersion":1,"agentCapabilities":{"sessionCapabilities":{"list":{},"delete":{}}},"authMethods":[]})).unwrap())
                }, agent_client_protocol::on_receive_request!())
                .on_receive_request(async |_: ListSessionsRequest, responder: Responder<ListSessionsResponse>, _: ConnectionTo<Client>| responder.respond(ListSessionsResponse::new(vec![])), agent_client_protocol::on_receive_request!())
                .on_receive_request(async |request: DeleteSessionRequest, responder: Responder<DeleteSessionResponse>, _: ConnectionTo<Client>| {
                    if request.session_id.to_string() == "fails" { responder.respond_with_error(agent_client_protocol::Error::internal_error().data("Synthetic failure")) }
                    else { responder.respond(DeleteSessionResponse::new()) }
                }, agent_client_protocol::on_receive_request!());
            DynConnectTo::new(agent)
        }
    }
    fn fixture() -> (tauri::App<MockRuntime>, Arc<Mutex<Vec<String>>>) {
        let visits = Arc::new(Mutex::new(Vec::new()));
        let app = crate::configure_shell(
            tauri::test::mock_builder().manage(crate::test_support::state()),
            crate::platform::Presentation(Arc::new(crate::test_support::UnusedPresentation)),
            crate::ui::TrayPresentation(Arc::new(crate::test_support::UnusedTray)),
            crate::agent::AgentServices(Arc::new(FixtureHost {
                visits: visits.clone(),
            })),
        )
        .build(crate::product_context())
        .unwrap();
        (app, visits)
    }
    fn local(
        store: &crate::session_history_store::HistoryStore,
        source: &HistorySource,
        cwd: &str,
        id: &str,
    ) {
        store
            .record_local(
                source,
                &StoredEntry {
                    session_id: id.into(),
                    cwd: cwd.into(),
                    title: None,
                    provider_updated_at: None,
                    local_activity_at: Some("invalid".into()),
                    state: EntryState::LocalOnly,
                    can_load: None,
                },
            )
            .unwrap();
    }

    fn fail_history_refresh(app: &tauri::App<MockRuntime>) {
        app.state::<AppState>()
            .session_view
            .poison_history_refresh_notice_for_test();
    }

    #[tokio::test]
    async fn delete_all_preserves_all_scope_outcomes_when_display_refresh_fails() {
        let (app, visits) = fixture();
        let state = app.state::<AppState>();
        let store = history_catalog::store(&state).unwrap();
        let source = history_catalog::source(&state.config().unwrap(), AgentKind::Codex).unwrap();
        local(&store, &source, "/first", "one");
        local(&store, &source, "/second", "two");
        local(&store, &source, "/offline", "retain");
        fail_history_refresh(&app);

        let result = delete_all_history(app.handle().clone(), store.current_revision().unwrap())
            .await
            .unwrap();

        assert_eq!((result.succeeded, result.failed, result.purged), (2, 1, 0));
        let rows = store.records().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry.session_id, "retain");
        assert_eq!(visits.lock().unwrap().len(), 3);
        assert_eq!(
            result
                .errors
                .iter()
                .filter(|error| error.starts_with("History menu refresh failed:"))
                .count(),
            1,
        );
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("Synthetic offline")));
    }

    #[tokio::test]
    async fn sync_preserves_refresh_purge_and_failure_outcomes_when_display_refresh_fails() {
        let (app, visits) = fixture();
        let state = app.state::<AppState>();
        let store = history_catalog::store(&state).unwrap();
        let source = history_catalog::source(&state.config().unwrap(), AgentKind::Codex).unwrap();
        local(&store, &source, "/first", "one");
        local(&store, &source, "/second", "two");
        local(&store, &source, "/offline", "retain");
        let orphan = HistorySource {
            invocation: "obsolete".into(),
            ..source.clone()
        };
        local(&store, &orphan, "/orphan", "purge");
        fail_history_refresh(&app);

        let result = sync_history(app.handle().clone()).await.unwrap();

        assert_eq!((result.succeeded, result.failed, result.purged), (2, 1, 1));
        assert!(store
            .records()
            .unwrap()
            .iter()
            .all(|row| row.source != orphan));
        assert!(store
            .entries(&source, "/offline")
            .unwrap()
            .iter()
            .any(|row| row.session_id == "retain"));
        assert_eq!(visits.lock().unwrap().len(), 3);
        assert_eq!(
            result
                .errors
                .iter()
                .filter(|error| error.starts_with("History menu refresh failed:"))
                .count(),
            1,
        );
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("Synthetic offline")));
    }
    #[tokio::test]
    async fn sync_visits_all_persisted_directories_and_empty_scopes_and_purges_only_orphans() {
        let (app, visits) = fixture();
        let state = app.state::<AppState>();
        let store = history_catalog::store(&state).unwrap();
        let source = history_catalog::source(&state.config().unwrap(), AgentKind::Codex).unwrap();
        local(&store, &source, "/elsewhere", "one");
        local(&store, &source, "/offline", "retain");
        store.begin_scan(&source, "/empty").unwrap();
        let orphan = HistorySource {
            agent: AgentKind::External(uuid::Uuid::new_v4()),
            invocation: "lost-definition".into(),
        };
        local(&store, &orphan, "/orphan", "purge");
        let result = sync_history(app.handle().clone()).await.unwrap();
        assert_eq!((result.succeeded, result.failed, result.purged), (2, 1, 1));
        let mut visited = visits.lock().unwrap().clone();
        visited.sort();
        assert_eq!(visited, vec!["/elsewhere", "/empty", "/offline"]);
        assert!(store.entries(&orphan, "/orphan").unwrap().is_empty());
        assert_eq!(store.entries(&source, "/offline").unwrap().len(), 1);
        let page = list_history(app.handle().clone(), 0, 50).await.unwrap();
        assert_eq!(page.total, 2);
        assert!(page.entries.iter().all(|entry| entry.updated_at.is_none()));
    }
    #[tokio::test]
    async fn delete_all_keeps_failed_rows_and_rejects_stale_revision_before_transport() {
        let (app, visits) = fixture();
        let state = app.state::<AppState>();
        let store = history_catalog::store(&state).unwrap();
        let source = history_catalog::source(&state.config().unwrap(), AgentKind::Codex).unwrap();
        local(&store, &source, "/other", "ok");
        local(&store, &source, "/other", "fails");
        let old = store.current_revision().unwrap();
        local(&store, &source, "/third", "newer");
        assert!(delete_all_history(app.handle().clone(), old)
            .await
            .unwrap_err()
            .contains("changed"));
        assert!(visits.lock().unwrap().is_empty());
        let result = delete_all_history(app.handle().clone(), store.current_revision().unwrap())
            .await
            .unwrap();
        assert_eq!((result.succeeded, result.failed), (2, 1));
        let rows = store.records().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry.session_id, "fails");
    }

    #[tokio::test]
    async fn errors_identify_each_agent_in_shared_working_directories() {
        let (app, _) = fixture();
        let state = app.state::<AppState>();
        let config = state.config().unwrap();
        let store = history_catalog::store(&state).unwrap();
        for agent in [AgentKind::Claude, AgentKind::Codex] {
            let source = history_catalog::source(&config, agent).unwrap();
            local(&store, &source, "/offline", "unreachable");
            local(&store, &source, "/shared", "fails");
        }
        let sync = sync_history(app.handle().clone()).await.unwrap();
        assert_eq!((sync.succeeded, sync.failed), (2, 2));
        for label in ["Claude Code", "ChatGPT Codex"] {
            assert!(sync
                .errors
                .iter()
                .any(|error| error.starts_with(&format!("{label} · /offline:"))));
        }
        let deleted = delete_all_history(app.handle().clone(), store.current_revision().unwrap())
            .await
            .unwrap();
        assert_eq!((deleted.succeeded, deleted.failed), (0, 4));
        for label in ["Claude Code", "ChatGPT Codex"] {
            assert!(deleted
                .errors
                .iter()
                .any(|error| error.starts_with(&format!("{label} · /shared: fails:"))));
        }
    }
    #[tokio::test]
    async fn admitted_management_blocks_live_start_until_lease_drops() {
        let (app, _) = fixture();
        let state = app.state::<AppState>();
        state
            .session_view
            .management_active
            .store(true, Ordering::Release);
        let lease = ManagementLease(&state.session_view.management_active);
        let mut lens = state.lens().unwrap();
        lens.stage = crate::model::LensStage::Connecting;
        assert!(
            crate::app_state::publish_lens_state(app.handle(), lens.clone())
                .unwrap_err()
                .contains("history management")
        );
        drop(lease);
        crate::app_state::publish_lens_state(app.handle(), lens).unwrap();
        assert!(sync_history(app.handle().clone())
            .await
            .unwrap_err()
            .contains("active session"));
    }
}
