//! Production admission and chronology checks; unexpected provider/native work panics.
use super::*;
use crate::{model::*, platform, test_support};
use std::sync::Arc;
use tauri::test::MockRuntime;

fn app(state: AppState) -> tauri::App<MockRuntime> {
    crate::configure_shell(
        tauri::test::mock_builder(),
        state,
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
        cwd: PathBuf::from("/fixture"),
        generation,
        entries: vec![HistoryEntry {
            agent: AgentKind::Codex,
            session_id: "foreign-session".into(),
            cwd: "/fixture".into(),
            title: "Other app".into(),
            updated_at: None,
            can_load: false,
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
        cwd: "/fixture".into(),
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
        Path::new("/fixture"),
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
                safe_mode_id: "unused",
                command: "/must-not-spawn".into(),
                args: vec![],
                installation: None,
            }))
        })
    }

    fn connect(
        &self,
        _: &crate::agent::AgentDescriptor,
    ) -> agent_client_protocol::DynConnectTo<agent_client_protocol::Client> {
        use agent_client_protocol::{
            schema::v1::{
                ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
                LoadSessionRequest, LoadSessionResponse, SessionNotification, SessionUpdate,
                TextContent,
            },
            Agent, Client, ConnectionTo, DynConnectTo, Responder,
        };
        let entered = Arc::clone(&self.entered);
        let release = Arc::clone(&self.release);
        let loaded = Arc::clone(&self.loaded);
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
                async move |request: LoadSessionRequest,
                            responder: Responder<LoadSessionResponse>,
                            cx: ConnectionTo<Client>| {
                    assert_eq!(request.session_id.to_string(), "external-codex-session");
                    assert_eq!(request.cwd, std::path::PathBuf::from("/fixture"));
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
    state.runtime.write().unwrap().config.working_directory = PathBuf::from("/fixture");
    let generation = Uuid::new_v4();
    *state.session_view.catalog.lock().unwrap() = HistoryCatalog {
        cwd: PathBuf::from("/fixture"),
        generation,
        entries: vec![HistoryEntry {
            agent: AgentKind::Codex,
            session_id: "external-codex-session".into(),
            cwd: "/fixture".into(),
            title: "Created outside Lens".into(),
            updated_at: Some("2026-09-17T00:00:00Z".into()),
            can_load: true,
        }],
        ..Default::default()
    };
    let app = crate::configure_shell(
        tauri::test::mock_builder(),
        state,
        platform::Presentation(Arc::new(test_support::UnusedPresentation)),
        crate::ui::TrayPresentation(Arc::new(ReplayTray)),
        crate::agent::AgentServices(host),
    )
    .build(crate::product_context())
    .unwrap();
    (app, generation)
}

fn replay_host() -> Arc<ReplayHost> {
    Arc::new(ReplayHost {
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
        policy_default: Some("read-only".into()),
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
        policy_default: Some("old-policy".into()),
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
    assert_eq!(next.policy_default, None);
}

#[test]
fn current_directory_filter_precedes_top_ten_and_excludes_descendants() {
    let mut entries = vec![];
    for index in 0..20 {
        let mut foreign = entry(&format!("foreign-{index}"), Some("2026-09-18T00:00:00Z"));
        foreign.cwd = if index % 2 == 0 {
            "/other"
        } else {
            "/fixture/child"
        }
        .into();
        entries.push(foreign);
    }
    entries.push(entry("current", Some("2026-09-17T00:00:00Z")));
    let catalog = merge_listings(
        Path::new("/fixture"),
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
    assert!(catalog.entries.iter().all(|e| e.cwd == "/fixture"));
}

#[tokio::test]
async fn different_directory_catalog_is_rejected_before_transport() {
    let (app, generation) = replay_app(replay_host());
    app.state::<AppState>()
        .runtime
        .write()
        .unwrap()
        .config
        .working_directory = PathBuf::from("/other");
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
            state.runtime.write().unwrap().config.working_directory = PathBuf::from("/other");
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
        PathBuf::from("/other")
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
        state.runtime.write().unwrap().config.working_directory = PathBuf::from("/other");
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
