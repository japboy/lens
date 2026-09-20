//! Production admission and chronology checks; unexpected provider/native work panics.
use super::*;
use crate::{model::*, platform, test_support};
use std::sync::Arc;
use tauri::test::MockRuntime;

fn app(state: AppState) -> tauri::App<MockRuntime> {
    crate::configure_shell(
        tauri::test::mock_builder().manage(state),
        platform::Presentation(Arc::new(test_support::UnusedPresentation)),
        crate::ui::TrayPresentation(Arc::new(test_support::UnusedTray)),
        crate::agent::AgentServices(Arc::new(test_support::UnusedAgent)),
    )
    .build(crate::product_context())
    .unwrap()
}

fn monitoring(lifecycle: LensMonitoringLifecycle) -> LensState {
    LensState {
        operation_id: Some(Uuid::new_v4()),
        stage: LensStage::Completed,
        live: Some(LensLiveState {
            lifecycle,
            health: LensSourceHealth::Healthy,
            freshness: LensFreshness::Current,
            agent_refresh_interval_seconds: 180,
            last_outcome: None,
            error: None,
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn watching_paused_and_progressing_operations_reject_history_before_effects() {
    for lens in [
        monitoring(LensMonitoringLifecycle::Watching),
        monitoring(LensMonitoringLifecycle::Paused),
        LensState {
            stage: LensStage::Extracting,
            ..Default::default()
        },
    ] {
        let state = test_support::state();
        state.runtime.write().unwrap().lens = lens;
        let app = app(state);
        assert!(!history_enabled(app.handle()).unwrap());
        let before = app.state::<AppState>().snapshot().unwrap();
        let error = open(app.handle().clone(), Uuid::new_v4(), 0)
            .await
            .unwrap_err();
        assert!(error.contains("active session"));
        let after = app.state::<AppState>().snapshot().unwrap();
        assert_eq!(after.config, before.config);
        assert_eq!(after.lens, before.lens);
        assert_eq!(
            app.state::<AppState>().session_view.view().unwrap().phase,
            ViewPhase::Idle
        );
    }
}

#[tokio::test]
async fn stale_catalog_and_unloadable_entries_never_resolve_an_agent() {
    let app = app(test_support::state());
    assert!(open(app.handle().clone(), Uuid::new_v4(), 0)
        .await
        .unwrap_err()
        .contains("changed"));
    let generation = Uuid::new_v4();
    *app.state::<AppState>().session_view.catalog.lock().unwrap() = HistoryCatalog {
        cwd: PathBuf::from("/tmp"),
        generation,
        entries: vec![HistoryEntry {
            agent: AgentKind::Codex,
            session_id: "foreign-session".into(),
            cwd: "/tmp".into(),
            title: "Other app".into(),
            updated_at: None,
            can_load: false,
            invocation: "managed-codex-v1".into(),
            presence: EntryState::Listed,
        }],
        ..Default::default()
    };
    assert!(open(app.handle().clone(), generation, 0)
        .await
        .unwrap_err()
        .contains("cannot load"));
    assert_eq!(
        app.state::<AppState>().session_view.view().unwrap().phase,
        ViewPhase::Idle
    );
}

#[test]
fn stale_failure_cannot_replace_new_generation_or_selection() {
    let app = app(test_support::state());
    let current = Uuid::new_v4();
    *app.state::<AppState>().session_view.inner.lock().unwrap() = SessionView {
        generation: current,
        phase: ViewPhase::Loading,
        agent: Some(AgentKind::Codex),
        ..Default::default()
    };
    let config = app.state::<AppState>().config().unwrap();
    fail(app.handle(), Uuid::new_v4(), "old failure".into()).unwrap();
    assert_eq!(
        app.state::<AppState>().session_view.view().unwrap().phase,
        ViewPhase::Loading
    );
    fail(app.handle(), current, "load failed".into()).unwrap();
    let view = app.state::<AppState>().session_view.view().unwrap();
    assert_eq!(view.phase, ViewPhase::Failed);
    assert_eq!(view.error.as_deref(), Some("load failed"));
    assert_eq!(app.state::<AppState>().config().unwrap(), config);
}

#[test]
fn history_close_cannot_stop_live_session_and_wrong_surface_is_denied() {
    let state = test_support::state();
    state.runtime.write().unwrap().lens = monitoring(LensMonitoringLifecycle::Paused);
    let app = app(state);
    let overlay =
        tauri::WebviewWindowBuilder::new(&app, crate::ui::LENS_WINDOW_LABEL, Default::default())
            .build()
            .unwrap();
    let before = app.state::<AppState>().lens().unwrap();
    let result = crate::shell_tests::invoke(&overlay, "close_session_view", serde_json::json!({}));
    assert!(result.unwrap_err().to_string().contains("live controls"));
    assert_eq!(app.state::<AppState>().lens().unwrap(), before);
    let settings = tauri::WebviewWindowBuilder::new(&app, "settings", Default::default())
        .build()
        .unwrap();
    let result = crate::shell_tests::invoke(&settings, "close_session_view", serde_json::json!({}));
    assert!(result.unwrap_err().to_string().contains("only available"));
}

fn listing(
    agent: AgentKind,
    entries: Vec<crate::session_history::HistorySessionEntry>,
) -> ProviderHistoryListing {
    ProviderHistoryListing {
        agent,
        entries,
        error: None,
        complete: true,
        can_load: true,
    }
}
fn entry(id: &str, timestamp: Option<&str>) -> crate::session_history::HistorySessionEntry {
    crate::session_history::HistorySessionEntry {
        session_id: id.into(),
        cwd: "/tmp".into(),
        title: Some(id.into()),
        updated_at: timestamp.map(str::to_owned),
    }
}

#[test]
fn compound_identity_top_ten_offsets_and_partial_unknown_are_explicit() {
    let mut claude_entries = vec![
        entry("same", Some("2026-09-17T10:00:00+09:00")),
        entry("unknown", None),
        entry("invalid", Some("invalid")),
    ];
    let mut codex_entries = vec![entry("same", Some("2026-09-17T00:59:00Z"))];
    for index in 0..12 {
        codex_entries.push(entry(
            &format!("older-{index:02}"),
            Some(&format!("2026-09-16T00:{index:02}:00Z")),
        ));
    }
    claude_entries.push(entry("same", Some("2026-09-17T10:00:00+09:00")));
    let mut partial = listing(AgentKind::Claude, claude_entries);
    partial.complete = false;
    partial.error = Some("page failed".into());
    let catalog = merge_listings(
        Path::new("/tmp"),
        vec![
            (AgentKind::Claude, Ok(partial)),
            (
                AgentKind::Codex,
                Ok(listing(AgentKind::Codex, codex_entries)),
            ),
        ],
    );
    assert_eq!(catalog.entries.len(), RECENT_SESSION_COUNT);
    assert_eq!(catalog.entries[0].agent, AgentKind::Claude);
    assert_eq!(catalog.entries[1].agent, AgentKind::Codex);
    assert_eq!(
        catalog
            .entries
            .iter()
            .filter(|entry| entry.session_id == "same")
            .count(),
        2
    );
    assert!(catalog
        .notices
        .iter()
        .any(|notice| notice.contains("partial history")));
    assert!(catalog
        .notices
        .iter()
        .any(|notice| notice.contains("page failed")));
    assert!(catalog
        .notices
        .iter()
        .any(|notice| notice.contains("2 sessions")));
    assert!(catalog
        .entries
        .iter()
        .all(|entry| timestamp(&entry.updated_at).is_some()));
    assert!(catalog
        .entries
        .windows(2)
        .all(|pair| timestamp(&pair[0].updated_at) >= timestamp(&pair[1].updated_at)));
}

#[tokio::test]
async fn history_loading_blocks_authentication_logout_and_reauthentication_before_effects() {
    for authenticating in [true, false] {
        let state = test_support::state();
        state.runtime.write().unwrap().agent_selection = AgentSelectionState {
            candidate: Some(AgentKind::Codex),
            stage: if authenticating {
                AgentSelectionStage::AuthenticationRequired
            } else {
                AgentSelectionStage::Selected
            },
            ..Default::default()
        };
        *state.session_view.inner.lock().unwrap() = SessionView {
            phase: ViewPhase::Loading,
            generation: Uuid::new_v4(),
            ..Default::default()
        };
        let app = app(state);
        let before = app.state::<AppState>().agent_selection().unwrap();
        if authenticating {
            assert!(
                crate::agent::authenticate_selection(app.handle().clone(), "fixture".into())
                    .await
                    .unwrap_err()
                    .contains("history")
            );
        } else {
            assert!(crate::agent::sign_out_selection(app.handle().clone())
                .await
                .unwrap_err()
                .contains("history"));
            assert!(crate::agent::reauthenticate_selection(app.handle().clone())
                .await
                .unwrap_err()
                .contains("history"));
        }
        assert_eq!(app.state::<AppState>().agent_selection().unwrap(), before);
        assert_eq!(
            app.state::<AppState>().session_view.view().unwrap().phase,
            ViewPhase::Loading
        );
    }
}

/// This host admits only installed-runtime resolution and initialize/load. Any
/// installation, generation or native-source access still fails loudly.
struct ReplayHost {
    empty_listing: bool,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    loaded: Arc<std::sync::atomic::AtomicUsize>,
}

impl crate::agent::AgentHost<MockRuntime> for ReplayHost {
    fn resolve<'a>(
        &'a self,
        _: &'a tauri::AppHandle<MockRuntime>,
        _: AgentKind,
    ) -> crate::agent::HostFuture<'a, crate::agent_runtime::ResolvedAgentRuntime> {
        panic!("history must not install or resolve a new runtime")
    }

    fn resolve_installed<'a>(
        &'a self,
        _: &'a tauri::AppHandle<MockRuntime>,
        kind: AgentKind,
    ) -> crate::agent::HostFuture<'a, Option<crate::agent_runtime::ResolvedAgentRuntime>> {
        assert_eq!(kind, AgentKind::Codex);
        Box::pin(async move {
            Ok(Some(crate::agent_runtime::ResolvedAgentRuntime {
                kind,
                adapter_name: "fixture-codex",
                adapter_version: "1.0.0".into(),
                command: "/must-not-spawn".into(),
                args: vec![],
                installation: None,
            }))
        })
    }

    fn connect(
        &self,
        _: &crate::agent::AgentDescriptor,
        cwd: std::path::PathBuf,
        purpose: crate::agent_environment::EnvironmentPurpose,
    ) -> agent_client_protocol::DynConnectTo<agent_client_protocol::Client> {
        use agent_client_protocol::{
            schema::v1::{
                ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
                ListSessionsRequest, ListSessionsResponse, LoadSessionRequest, LoadSessionResponse,
                SessionInfo, SessionNotification, SessionUpdate, TextContent,
            },
            Agent, Client, ConnectionTo, DynConnectTo, Responder,
        };
        assert_eq!(
            purpose,
            crate::agent_environment::EnvironmentPurpose::History
        );
        let entered = Arc::clone(&self.entered);
        let release = Arc::clone(&self.release);
        let loaded = Arc::clone(&self.loaded);
        let listed_title = "Renamed elsewhere".to_string();
        let empty_listing = self.empty_listing;
        let fixture = Agent.builder()
            .on_receive_request(
                async |_request: InitializeRequest,
                       responder: Responder<InitializeResponse>,
                       _cx: ConnectionTo<Client>| {
                    responder.respond(serde_json::from_value(serde_json::json!({
                        "protocolVersion": 1,
                        "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"list": {}}},
                        "authMethods": []
                    })).unwrap())
                }, agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: ListSessionsRequest,
                            responder: Responder<ListSessionsResponse>,
                            _: ConnectionTo<Client>| {
                    assert_eq!(request.cwd, Some(PathBuf::from("/tmp")));
                    responder.respond(ListSessionsResponse::new(if empty_listing { vec![] } else { vec![SessionInfo::new(
                        "external-codex-session", "/tmp",
                    ).title(listed_title.clone()).updated_at("2026-09-20T00:00:00Z")] }))
                }, agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: LoadSessionRequest,
                            responder: Responder<LoadSessionResponse>,
                            cx: ConnectionTo<Client>| {
                    assert_eq!(request.session_id.to_string(), "external-codex-session");
                    assert_eq!(request.cwd, cwd);
                    assert_eq!(request.cwd, std::path::PathBuf::from("/tmp"));
                    assert!(request.mcp_servers.is_empty());
                    loaded.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    cx.send_notification(SessionNotification::new(
                        request.session_id,
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("Replayed external session"),
                        ))),
                    ))?;
                    entered.notify_one();
                    release.notified().await;
                    responder.respond(LoadSessionResponse::new())
                }, agent_client_protocol::on_receive_request!(),
            );
        DynConnectTo::new(fixture)
    }
}

struct ReplayTray;
impl crate::ui::TrayOutput<MockRuntime> for ReplayTray {
    fn apply(
        &self,
        _: &tauri::AppHandle<MockRuntime>,
        _: crate::ui::TrayMenuPresentation,
    ) -> Result<(), String> {
        Ok(())
    }
}

fn replay_app(host: Arc<ReplayHost>) -> (tauri::App<MockRuntime>, Uuid) {
    let state = test_support::state();
    state.runtime.write().unwrap().config.agent = AgentKind::Claude;
    state.runtime.write().unwrap().config.working_directory = PathBuf::from("/tmp");
    let source = history_catalog::source(&state.config().unwrap(), AgentKind::Codex).unwrap();
    let store = history_catalog::store(&state).unwrap();
    for id in ["external-codex-session", "not-returned"] {
        store
            .record_local(
                &source,
                &StoredEntry {
                    session_id: id.into(),
                    cwd: "/tmp".into(),
                    title: Some("Created outside Lens".into()),
                    provider_updated_at: Some("2026-09-17T00:00:00Z".into()),
                    local_activity_at: None,
                    state: EntryState::LocalOnly,
                    can_load: Some(true),
                },
            )
            .unwrap();
    }
    let generation = Uuid::new_v4();
    *state.session_view.catalog.lock().unwrap() = HistoryCatalog {
        cwd: PathBuf::from("/tmp"),
        generation,
        entries: vec![HistoryEntry {
            agent: AgentKind::Codex,
            session_id: "external-codex-session".into(),
            cwd: "/tmp".into(),
            title: "Created outside Lens".into(),
            updated_at: Some("2026-09-17T00:00:00Z".into()),
            can_load: true,
            invocation: history_catalog::source(&state.config().unwrap(), AgentKind::Codex)
                .unwrap()
                .invocation,
            presence: EntryState::Listed,
        }],
        ..Default::default()
    };
    let app = crate::configure_shell(
        tauri::test::mock_builder().manage(state),
        platform::Presentation(Arc::new(test_support::UnusedPresentation)),
        crate::ui::TrayPresentation(Arc::new(ReplayTray)),
        crate::agent::AgentServices(host),
    )
    .build(crate::product_context())
    .unwrap();
    // Replay lifecycle tests do not exercise native monitor placement.
    tauri::WebviewWindowBuilder::new(
        &app,
        crate::ui::LENS_WINDOW_LABEL,
        tauri::WebviewUrl::App("overlay.html".into()),
    )
    .build()
    .unwrap();
    (app, generation)
}

fn replay_host() -> Arc<ReplayHost> {
    Arc::new(ReplayHost {
        empty_listing: false,
        entered: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
        loaded: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    })
}

#[tokio::test]
async fn successful_replay_commits_provider_and_ready_document_only_after_response() {
    let host = replay_host();
    let (app, catalog) = replay_app(host.clone());
    let original_lens = app.state::<AppState>().lens().unwrap();
    let opening = open(app.handle().clone(), catalog, 0);
    let inspect_loading = async {
        host.entered.notified().await;
        let state = app.state::<AppState>();
        assert_eq!(state.config().unwrap().agent, AgentKind::Claude);
        let view = state.session_view.view().unwrap();
        assert_eq!(view.phase, ViewPhase::Loading);
        assert!(view.document.is_none());
        assert_eq!(state.lens().unwrap(), original_lens);
        host.release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(opening, inspect_loading)
    })
    .await
    .unwrap();
    result.unwrap();
    let state = app.state::<AppState>();
    assert_eq!(state.config().unwrap().agent, AgentKind::Codex);
    assert_eq!(state.store.load().agent, AgentKind::Codex);
    assert_eq!(
        state.agent_selection().unwrap().stage,
        AgentSelectionStage::HistorySelected
    );
    assert!(!state.agent_selection().unwrap().can_select_lens_target());
    assert_eq!(state.lens().unwrap(), original_lens);
    assert!(!active_session(&state.lens().unwrap()));
    let view = state.session_view.view().unwrap();
    assert_eq!(view.phase, ViewPhase::Ready);
    assert_eq!(view.title.as_deref(), Some("Renamed elsewhere"));
    let source = history_catalog::source(&state.config().unwrap(), AgentKind::Codex).unwrap();
    let rows = history_catalog::store(&state)
        .unwrap()
        .entries(&source, "/tmp")
        .unwrap();
    assert_eq!(
        rows.iter()
            .find(|row| row.session_id == "external-codex-session")
            .unwrap()
            .title
            .as_deref(),
        Some("Renamed elsewhere")
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.session_id == "not-returned")
            .unwrap()
            .state,
        EntryState::NotSeen
    );
    assert_eq!(view.session_id.as_deref(), Some("external-codex-session"));
    assert_eq!(host.loaded.load(std::sync::atomic::Ordering::SeqCst), 1);
    let overlay = app
        .get_webview_window(crate::ui::LENS_WINDOW_LABEL)
        .unwrap();
    let rendered =
        crate::shell_tests::invoke(&overlay, "get_session_view", serde_json::json!({})).unwrap();
    assert_eq!(rendered["phase"], "ready");
    assert_eq!(
        rendered["document"]["entries"][0]["blocks"][0]["text"],
        "Replayed external session"
    );
}

#[tokio::test]
async fn closing_loading_history_invalidates_late_success_without_selecting_provider() {
    let host = replay_host();
    let (app, catalog) = replay_app(host.clone());
    let original_lens = app.state::<AppState>().lens().unwrap();
    let opening = open(app.handle().clone(), catalog, 0);
    let close_loading = async {
        host.entered.notified().await;
        let overlay = app
            .get_webview_window(crate::ui::LENS_WINDOW_LABEL)
            .unwrap();
        crate::shell_tests::invoke(&overlay, "close_session_view", serde_json::json!({})).unwrap();
        assert_eq!(
            app.state::<AppState>().session_view.view().unwrap().phase,
            ViewPhase::Idle
        );
        host.release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(opening, close_loading)
    })
    .await
    .unwrap();
    result.unwrap();
    let state = app.state::<AppState>();
    assert_eq!(state.config().unwrap().agent, AgentKind::Claude);
    assert_eq!(state.lens().unwrap(), original_lens);
    let view = state.session_view.view().unwrap();
    assert_eq!(view.phase, ViewPhase::Idle);
    assert!(view.document.is_none());
    assert_eq!(host.loaded.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn same_provider_history_preserves_selection_identity_and_settings_catalog() {
    let current = AgentSelectionState {
        operation_id: Some(Uuid::new_v4()),
        candidate: Some(AgentKind::Codex),
        stage: AgentSelectionStage::Selected,
        config_options: Some(vec![]),
        agent_default: Some("read-only".into()),
        message: Some("Verified".into()),
        ..Default::default()
    };
    assert_eq!(selection_after_history(&current, AgentKind::Codex), current);
}

#[test]
fn cross_provider_history_choice_does_not_claim_live_readiness() {
    let current = AgentSelectionState {
        candidate: Some(AgentKind::Claude),
        stage: AgentSelectionStage::Selected,
        operation_id: Some(Uuid::new_v4()),
        agent_default: Some("old-policy".into()),
        ..Default::default()
    };
    let next = selection_after_history(&current, AgentKind::Codex);
    assert_eq!(next.stage, AgentSelectionStage::HistorySelected);
    assert_eq!(next.candidate, Some(AgentKind::Codex));
    assert!(next.operation_id.is_some());
    assert_ne!(next.operation_id, current.operation_id);
    assert_eq!(next.selected_agent(), None);
    assert!(!next.can_select_lens_target());
    assert_eq!(next.config_options, None);
    assert_eq!(next.agent_default, None);
}

#[test]
fn current_directory_filter_precedes_top_ten_and_excludes_descendants() {
    let mut entries = vec![];
    for index in 0..20 {
        let mut foreign = entry(&format!("foreign-{index}"), Some("2026-09-18T00:00:00Z"));
        foreign.cwd = if index % 2 == 0 {
            "/"
        } else {
            "/fixture/child"
        }
        .into();
        entries.push(foreign);
    }
    entries.push(entry("current", Some("2026-09-17T00:00:00Z")));
    let catalog = merge_listings(
        Path::new("/tmp"),
        vec![
            (AgentKind::Codex, Ok(listing(AgentKind::Codex, entries))),
            (
                AgentKind::Claude,
                Ok(listing(
                    AgentKind::Claude,
                    vec![entry("current", Some("2026-09-16T00:00:00Z"))],
                )),
            ),
        ],
    );
    assert_eq!(catalog.entries.len(), 2);
    assert!(catalog.entries.iter().all(|e| e.cwd == "/tmp"));
}

#[tokio::test]
async fn different_directory_catalog_is_rejected_before_transport() {
    let (app, generation) = replay_app(replay_host());
    app.state::<AppState>()
        .runtime
        .write()
        .unwrap()
        .config
        .working_directory = PathBuf::from("/");
    assert!(open(app.handle().clone(), generation, 0)
        .await
        .unwrap_err()
        .contains("Working Directory changed"));
    assert_eq!(
        app.state::<AppState>().session_view.view().unwrap().phase,
        ViewPhase::Idle
    );
}

#[tokio::test]
async fn changing_directory_invalidates_late_replay_without_switching_agent() {
    let host = replay_host();
    let (app, catalog) = replay_app(host.clone());
    let opening = open(app.handle().clone(), catalog, 0);
    let change = async {
        host.entered.notified().await;
        {
            let state = app.state::<AppState>();
            let _guard = state.session_view.admission.lock().unwrap();
            state.runtime.write().unwrap().config.working_directory = PathBuf::from("/");
            invalidate_working_directory(app.handle()).unwrap();
        }
        host.release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(opening, change)
    })
    .await
    .unwrap();
    result.unwrap();
    let state = app.state::<AppState>();
    assert_eq!(state.config().unwrap().agent, AgentKind::Claude);
    assert_eq!(
        state.config().unwrap().working_directory,
        PathBuf::from("/")
    );
    assert_eq!(state.session_view.view().unwrap().phase, ViewPhase::Idle);
    assert!(state.session_view.catalog().unwrap().entries.is_empty());
    assert_ne!(state.session_view.catalog().unwrap().generation, catalog);
}

#[test]
fn directory_change_rejects_late_catalog_even_after_returning_to_original_directory() {
    let (app, _) = replay_app(replay_host());
    let state = app.state::<AppState>();
    let old = state.session_view.catalog().unwrap();
    {
        let _guard = state.session_view.admission.lock().unwrap();
        state.runtime.write().unwrap().config.working_directory = PathBuf::from("/");
        invalidate_working_directory(app.handle()).unwrap();
    }
    commit_catalog(app.handle(), old.clone()).unwrap();
    assert!(state.session_view.catalog().unwrap().entries.is_empty());
    {
        let _guard = state.session_view.admission.lock().unwrap();
        state.runtime.write().unwrap().config.working_directory = old.cwd.clone();
        invalidate_working_directory(app.handle()).unwrap();
    }
    commit_catalog(app.handle(), old).unwrap();
    assert!(state.session_view.catalog().unwrap().entries.is_empty());
}

#[test]
fn same_session_ids_from_different_external_profiles_remain_distinct() {
    let listings = [Uuid::from_u128(1), Uuid::from_u128(2)]
        .into_iter()
        .map(|id| {
            let agent = AgentKind::External(id);
            (
                agent,
                Ok(ProviderHistoryListing {
                    agent,
                    complete: true,
                    can_load: true,
                    error: None,
                    entries: vec![crate::session_history::HistorySessionEntry {
                        session_id: "shared-id".into(),
                        cwd: "/tmp".into(),
                        title: Some("Same title".into()),
                        updated_at: Some("2026-09-17T00:00:00Z".into()),
                    }],
                }),
            )
        })
        .collect();
    let catalog = merge_listings(Path::new("/tmp"), listings);
    assert_eq!(catalog.entries.len(), 2);
    assert_ne!(catalog.entries[0].agent, catalog.entries[1].agent);
    assert_eq!(catalog.entries[0].session_id, catalog.entries[1].session_id);
}

struct DiscoveryHost {
    resolved: Arc<Mutex<Vec<AgentKind>>>,
    mutate_profile: bool,
}

impl crate::agent::AgentHost<MockRuntime> for DiscoveryHost {
    fn resolve<'a>(
        &'a self,
        _: &'a tauri::AppHandle<MockRuntime>,
        _: AgentKind,
    ) -> crate::agent::HostFuture<'a, crate::agent_runtime::ResolvedAgentRuntime> {
        panic!("history must not install an Agent")
    }

    fn resolve_installed<'a>(
        &'a self,
        app: &'a tauri::AppHandle<MockRuntime>,
        kind: AgentKind,
    ) -> crate::agent::HostFuture<'a, Option<crate::agent_runtime::ResolvedAgentRuntime>> {
        self.resolved.lock().unwrap().push(kind);
        Box::pin(async move {
            if self.mutate_profile && kind.is_external() {
                app.state::<AppState>()
                    .runtime
                    .write()
                    .unwrap()
                    .config
                    .external_agents[0]
                    .args
                    .push("changed".into());
                Ok(Some(crate::agent_runtime::ResolvedAgentRuntime {
                    kind,
                    adapter_name: "fixture",
                    adapter_version: "1".into(),
                    command: "/must-not-spawn".into(),
                    args: vec![],
                    installation: None,
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn connect(
        &self,
        _: &crate::agent::AgentDescriptor,
        _: std::path::PathBuf,
        _: crate::agent_environment::EnvironmentPurpose,
    ) -> agent_client_protocol::DynConnectTo<agent_client_protocol::Client> {
        use agent_client_protocol::{schema::v1::InitializeRequest, Agent, DynConnectTo};
        let fixture = Agent.builder().on_receive_request(
            async |_: InitializeRequest,
                   _: agent_client_protocol::Responder<
                agent_client_protocol::schema::v1::InitializeResponse,
            >,
                   _: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>|
                   -> Result<(), agent_client_protocol::Error> {
                panic!("a changed external profile must not be connected")
            },
            agent_client_protocol::on_receive_request!(),
        );
        DynConnectTo::new(fixture)
    }
}

#[tokio::test]
async fn history_refresh_never_resolves_agents_for_any_selection_stage() {
    let state = test_support::state();
    let external = AgentKind::External(state.config().unwrap().external_agents[0].id);
    let resolved = Arc::new(Mutex::new(Vec::new()));
    let app = crate::configure_shell(
        tauri::test::mock_builder().manage(state),
        platform::Presentation(Arc::new(test_support::UnusedPresentation)),
        crate::ui::TrayPresentation(Arc::new(test_support::UnusedTray)),
        crate::agent::AgentServices(Arc::new(DiscoveryHost {
            resolved: Arc::clone(&resolved),
            mutate_profile: false,
        })),
    )
    .build(crate::product_context())
    .unwrap();
    for stage in [
        AgentSelectionStage::Unselected,
        AgentSelectionStage::Failed,
        AgentSelectionStage::HistorySelected,
        AgentSelectionStage::Selected,
    ] {
        {
            let state = app.state::<AppState>();
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.config.agent = external;
            snapshot.agent_selection.candidate = Some(external);
            snapshot.agent_selection.stage = stage;
        }
        refresh(app.handle().clone()).await.unwrap();
        let observed = std::mem::take(&mut *resolved.lock().unwrap());
        assert!(
            observed.is_empty(),
            "cached refresh unexpectedly resolved {observed:?}"
        );
    }
}

#[tokio::test]
async fn external_history_rejects_profile_changes_during_runtime_resolution() {
    let state = test_support::state();
    let kind = AgentKind::External(state.config().unwrap().external_agents[0].id);
    {
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.config.agent = kind;
        snapshot.agent_selection.candidate = Some(kind);
        snapshot.agent_selection.stage = AgentSelectionStage::Selected;
    }
    let app = crate::configure_shell(
        tauri::test::mock_builder().manage(state),
        platform::Presentation(Arc::new(test_support::UnusedPresentation)),
        crate::ui::TrayPresentation(Arc::new(test_support::UnusedTray)),
        crate::agent::AgentServices(Arc::new(DiscoveryHost {
            resolved: Arc::new(Mutex::new(Vec::new())),
            mutate_profile: true,
        })),
    )
    .build(crate::product_context())
    .unwrap();
    let error = session_history::load_synced_provider(
        app.handle(),
        kind,
        &app.state::<AppState>().config().unwrap(),
        "existing",
        "/tmp",
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("configuration changed"));
}

fn seed_cached(state: &AppState, agent: AgentKind, id: &str, cwd: &str, time: &str) {
    let source = history_catalog::source(&state.config().unwrap(), agent).unwrap();
    history_catalog::store(state)
        .unwrap()
        .record_local(
            &source,
            &StoredEntry {
                session_id: id.into(),
                cwd: cwd.into(),
                title: Some(id.into()),
                provider_updated_at: Some(time.into()),
                local_activity_at: None,
                state: EntryState::LocalOnly,
                can_load: None,
            },
        )
        .unwrap();
}

#[tokio::test]
async fn cached_all_agents_preserve_selection_and_never_connect() {
    let state = test_support::state();
    state.runtime.write().unwrap().config.working_directory = PathBuf::from("/tmp");
    let external = AgentKind::External(state.config().unwrap().external_agents[0].id);
    for agent in [AgentKind::Claude, AgentKind::Codex, external] {
        seed_cached(&state, agent, "shared-id", "/tmp", "2026-09-20T00:00:00Z");
    }
    let app = app(state); // UnusedAgent panics on every attempted Agent effect.
    let before = app.state::<AppState>().snapshot().unwrap();
    refresh(app.handle().clone()).await.unwrap();
    let catalog = app.state::<AppState>().session_view.catalog().unwrap();
    assert_eq!(catalog.entries.len(), 3);
    assert!(catalog
        .entries
        .iter()
        .all(|entry| entry.session_id == "shared-id"));
    let after = app.state::<AppState>().snapshot().unwrap();
    assert_eq!(after.config, before.config);
    assert_eq!(after.agent_selection, before.agent_selection);
    assert_eq!(after.lens, before.lens);
}

#[test]
fn cached_directory_filter_precedes_global_top_ten() {
    let state = test_support::state();
    for index in 0..12 {
        seed_cached(
            &state,
            AgentKind::Claude,
            &format!("new-{index}"),
            "/tmp",
            "2026-09-20T00:00:00Z",
        );
        seed_cached(
            &state,
            AgentKind::Codex,
            &format!("child-{index}"),
            "/tmp/child",
            "2026-09-21T00:00:00Z",
        );
    }
    seed_cached(
        &state,
        AgentKind::Codex,
        "older-but-selected",
        "/tmp",
        "2026-09-19T00:00:00Z",
    );
    let store = history_catalog::store(&state).unwrap();
    let sources = history_catalog::sources(&state.config().unwrap()).unwrap();
    let all = cached_catalog(&store, Path::new("/tmp"), sources).unwrap();
    assert_eq!(all.entries.len(), 10);
    assert!(all
        .entries
        .iter()
        .all(|entry| entry.agent == AgentKind::Claude));
}

#[tokio::test]
async fn edited_or_deleted_presets_reject_cached_open_before_effects() {
    for delete in [false, true] {
        let state = test_support::state();
        state.runtime.write().unwrap().config.working_directory = PathBuf::from("/tmp");
        let external = AgentKind::External(state.config().unwrap().external_agents[0].id);
        seed_cached(&state, external, "saved", "/tmp", "2026-09-20T00:00:00Z");
        let app = app(state);
        refresh(app.handle().clone()).await.unwrap();
        let generation = app
            .state::<AppState>()
            .session_view
            .catalog()
            .unwrap()
            .generation;
        {
            let state = app.state::<AppState>();
            let mut snapshot = state.runtime.write().unwrap();
            if delete {
                snapshot.config.external_agents.remove(0);
            } else {
                snapshot.config.external_agents[0]
                    .args
                    .push("changed".into());
            }
        }
        let error = open(app.handle().clone(), generation, 0).await.unwrap_err();
        assert!(error.contains(if delete {
            "no longer available"
        } else {
            "command changed"
        }));
        assert_eq!(
            app.state::<AppState>().session_view.view().unwrap().phase,
            ViewPhase::Idle
        );
    }
}

#[test]
fn live_info_updates_persist_metadata_but_stale_sessions_do_not() {
    use agent_client_protocol::schema::{v1::SessionInfoUpdate, MaybeUndefined};
    let state = test_support::state();
    let operation = Uuid::new_v4();
    {
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.config.working_directory = PathBuf::from("/tmp");
        snapshot.lens = monitoring(LensMonitoringLifecycle::Watching);
        snapshot.lens.operation_id = Some(operation);
    }
    let app = app(state);
    start_live(
        app.handle(),
        operation,
        AgentKind::Claude,
        "live-session".into(),
    )
    .unwrap();
    record_live(
        app.handle(),
        "live-session",
        SessionUpdate::SessionInfoUpdate(
            SessionInfoUpdate::new()
                .title("Agent title")
                .updated_at("2026-09-20T00:00:00Z"),
        ),
    )
    .unwrap();
    record_live(
        app.handle(),
        "different-session",
        SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title("Must not replace")),
    )
    .unwrap();
    let state = app.state::<AppState>();
    let source = history_catalog::source(&state.config().unwrap(), AgentKind::Claude).unwrap();
    let store = history_catalog::store(&state).unwrap();
    history_catalog::writer(&state).unwrap().flush().unwrap();
    let row = store.entries(&source, "/tmp").unwrap().remove(0);
    assert_eq!(row.title.as_deref(), Some("Agent title"));
    assert_eq!(
        row.provider_updated_at.as_deref(),
        Some("2026-09-20T00:00:00Z")
    );
    assert!(row.local_activity_at.is_some());
    record_live(
        app.handle(),
        "live-session",
        SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(MaybeUndefined::Null)),
    )
    .unwrap();
    history_catalog::writer(&state).unwrap().flush().unwrap();
    let row = store.entries(&source, "/tmp").unwrap().remove(0);
    assert_eq!(row.title, None);
    assert_eq!(
        row.provider_updated_at.as_deref(),
        Some("2026-09-20T00:00:00Z")
    );
}

#[test]
fn accepted_metadata_does_not_erase_a_previous_persistence_notice() {
    let app = app(test_support::state());
    report_history_write(app.handle(), Err("Metadata was not saved".into()));
    report_history_write(app.handle(), Ok(()));
    assert_eq!(
        app.state::<AppState>()
            .session_view
            .storage_notice
            .lock()
            .unwrap()
            .as_deref(),
        Some("Metadata was not saved")
    );
}

#[tokio::test]
async fn remote_scan_follows_all_previously_queued_live_metadata() {
    let state = test_support::state();
    let source = history_catalog::source(&state.config().unwrap(), AgentKind::Claude).unwrap();
    let writer = history_catalog::writer(&state).unwrap();
    writer
        .record_local(
            source.clone(),
            StoredEntry {
                session_id: "queued-session".into(),
                cwd: "/tmp".into(),
                title: None,
                provider_updated_at: None,
                local_activity_at: Some("2026-09-19T00:00:00Z".into()),
                state: EntryState::LocalOnly,
                can_load: None,
            },
        )
        .unwrap();
    writer
        .apply_info_patch(
            source.clone(),
            "queued-session".into(),
            "/tmp".into(),
            InfoPatch {
                title: Patch::Value("Previous live title".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let app = app(state);
    // This is the same barrier open awaits before admission and begin_scan.
    flush_history_writes(app.handle()).await.unwrap();
    let state = app.state::<AppState>();
    let store = history_catalog::store(&state).unwrap();
    assert_eq!(
        store.entries(&source, "/tmp").unwrap()[0].title.as_deref(),
        Some("Previous live title")
    );
    let token = store.begin_scan(&source, "/tmp").unwrap();
    store
        .apply_listing(
            &token,
            &[crate::session_history_store::ListedEntry {
                session_id: "queued-session".into(),
                title: Some("Fresh remote title".into()),
                provider_updated_at: Some("2026-09-20T00:00:00Z".into()),
            }],
            true,
            Some(true),
        )
        .unwrap();
    flush_history_writes(app.handle()).await.unwrap();
    assert_eq!(
        store.entries(&source, "/tmp").unwrap()[0].title.as_deref(),
        Some("Fresh remote title")
    );
}

#[tokio::test]
async fn preset_edit_after_admission_never_resolves_replacement_command() {
    let state = test_support::state();
    let admitted = state.config().unwrap();
    let kind = AgentKind::External(admitted.external_agents[0].id);
    let app = app(state);
    // Simulate the settings edit after the menu action released admission but
    // before its provider future is polled. UnusedAgent panics on resolution.
    app.state::<AppState>()
        .runtime
        .write()
        .unwrap()
        .config
        .external_agents[0]
        .command = "replacement-command".into();
    let result = session_history::load_synced_provider(
        app.handle(),
        kind,
        &admitted,
        "existing",
        admitted.working_directory.to_str().unwrap(),
    )
    .await;
    assert!(matches!(result, Err(error) if error.contains("configuration changed")));
}

#[tokio::test]
async fn scan_write_failures_preserve_cached_entries_and_display_notice() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite3");
    let mut state = test_support::state();
    state.history_writer.take();
    state.history_store.take();
    history_catalog::install(
        &state,
        crate::session_history_store::HistoryStore::open(&path).unwrap(),
    )
    .unwrap();
    state.runtime.write().unwrap().config.working_directory = PathBuf::from("/tmp");
    seed_cached(
        &state,
        AgentKind::Claude,
        "retained",
        "/tmp",
        "2026-09-20T00:00:00Z",
    );
    // Reads remain usable while every scan write deterministically fails.
    rusqlite::Connection::open(&path).unwrap().execute_batch(
        "CREATE TRIGGER reject_history_clock BEFORE UPDATE ON history_clock BEGIN SELECT RAISE(ABORT, 'fixture scan write failed'); END;"
    ).unwrap();
    let app = app(state);
    refresh(app.handle().clone()).await.unwrap();
    let catalog = app.state::<AppState>().session_view.catalog().unwrap();
    let error = open(app.handle().clone(), catalog.generation, 0)
        .await
        .unwrap_err();
    assert!(error.contains("fixture scan write failed"));
    let catalog = app.state::<AppState>().session_view.catalog().unwrap();
    assert_eq!(catalog.entries.len(), 1);
    assert_eq!(catalog.entries[0].session_id, "retained");
    assert!(catalog
        .notices
        .iter()
        .any(|notice| notice.contains("fixture scan write failed")));
}

#[tokio::test]
async fn successful_replay_clears_uncertain_availability_after_empty_listing() {
    let mut host = replay_host();
    Arc::get_mut(&mut host).unwrap().empty_listing = true;
    let (app, generation) = replay_app(host.clone());
    host.release.notify_one();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        open(app.handle().clone(), generation, 0),
    )
    .await
    .unwrap()
    .unwrap();
    let catalog = app.state::<AppState>().session_view.catalog().unwrap();
    let opened = catalog
        .entries
        .iter()
        .find(|entry| entry.session_id == "external-codex-session")
        .unwrap();
    assert_eq!(opened.presence, EntryState::LocalOnly);
    assert!(opened.can_load);
    assert_eq!(opened.updated_at.as_deref(), Some("2026-09-17T00:00:00Z"));
    assert_eq!(
        catalog
            .entries
            .iter()
            .find(|entry| entry.session_id == "not-returned")
            .unwrap()
            .presence,
        EntryState::NotSeen
    );
}
