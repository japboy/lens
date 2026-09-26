//! Managed update regressions use real ACP connections and isolated settings/selectors.
use super::*;
use crate::agent::{AgentDescriptor, AgentHost, AgentServices, HostFuture, VerifiedManagedRuntime};
use crate::agent_preferences::{AgentDefaults, SavedChoice, ToolPolicy};
use agent_client_protocol::{
    schema::{v1::*, ProtocolVersion},
    Agent, Client, DynConnectTo, Responder,
};
use std::sync::{atomic::AtomicUsize, Mutex};
use tauri::test::MockRuntime;
use tokio::sync::Notify;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Normal,
    Refuse,
    Malformed,
    Auth,
    Cascade,
    Empty,
    ChangedType,
    ResetMode,
    CyclicReset,
    ChangingCyclicReset,
    UnconfirmedValue,
    RepairRemovesModel,
}

struct Fixture {
    runtime: Mutex<ResolvedAgentRuntime>,
    scenario: Scenario,
    requests: Mutex<Vec<(String, String)>>,
    sessions: AtomicUsize,
    hold_next: AtomicBool,
    entered: Notify,
    release: Notify,
}
fn select(id: &str, name: &str, current: &str, values: &[&str]) -> SessionConfigOption {
    SessionConfigOption::select(
        id.to_owned(),
        name,
        current.to_owned(),
        values
            .iter()
            .map(|value| SessionConfigSelectOption::new((*value).to_owned(), *value))
            .collect::<Vec<_>>(),
    )
}
fn catalog() -> Vec<SessionConfigOption> {
    vec![
        select("model", "Model", "new", &["new", "kept"])
            .category(SessionConfigOptionCategory::Model),
        select("thought", "Reasoning effort", "normal", &["normal"])
            .category(SessionConfigOptionCategory::ThoughtLevel),
        select("advanced", "Detail", "a", &["a", "b"]),
    ]
}
fn old_catalog() -> Vec<SessionConfigOption> {
    vec![
        select(
            "model",
            "Previous Model label",
            "removed",
            &["removed", "kept"],
        )
        .category(SessionConfigOptionCategory::Model),
        select("thought", "Reasoning effort", "high", &["normal", "high"])
            .category(SessionConfigOptionCategory::ThoughtLevel),
        select("advanced", "Previous detail label", "a", &["a", "b"]),
    ]
}

fn defaults(choices: &[(&str, &str)]) -> AgentDefaults {
    let mut defaults = AgentDefaults::default();
    defaults.tools.execute = ToolPolicy::Ask;
    defaults.choices = choices
        .iter()
        .map(|(id, value)| SavedChoice {
            config_id: (*id).into(),
            value: (*value).into(),
        })
        .collect();
    defaults
}
impl AgentHost<MockRuntime> for Arc<Fixture> {
    fn resolve<'a>(
        &'a self,
        _: &'a AppHandle<MockRuntime>,
        _: AgentKind,
    ) -> HostFuture<'a, ResolvedAgentRuntime> {
        let runtime = self.runtime.lock().unwrap().clone();
        Box::pin(async { Ok(runtime) })
    }
    fn resolve_installed<'a>(
        &'a self,
        _: &'a AppHandle<MockRuntime>,
        _: AgentKind,
    ) -> HostFuture<'a, Option<ResolvedAgentRuntime>> {
        let runtime = self.runtime.lock().unwrap().clone();
        Box::pin(async { Ok(Some(runtime)) })
    }
    fn connect(
        &self,
        _: &AgentDescriptor,
        _: PathBuf,
        _: crate::agent_environment::EnvironmentPurpose,
    ) -> DynConnectTo<Client> {
        let initial = self.clone();
        let configure = self.clone();
        let catalogs = Arc::new(Mutex::new(std::collections::HashMap::<
            String,
            Vec<SessionConfigOption>,
        >::new()));
        let created = catalogs.clone();
        DynConnectTo::new(Agent.builder()
            .on_receive_request(
                async |_: InitializeRequest, responder: Responder<InitializeResponse>, _| {
                    responder.respond(InitializeResponse::new(ProtocolVersion::V1)
                        .agent_capabilities(AgentCapabilities::new().mcp_capabilities(McpCapabilities::new().http(true))))
                }, agent_client_protocol::on_receive_request!())
            .on_receive_request(
                async move |_: NewSessionRequest, responder: Responder<NewSessionResponse>, _| {
                    let index = initial.sessions.fetch_add(1, Ordering::SeqCst);
                    if initial.hold_next.swap(false, Ordering::SeqCst) {
                        initial.entered.notify_one();
                        initial.release.notified().await;
                    }
                    if initial.scenario == Scenario::Auth {
                        return responder.respond_with_error(agent_client_protocol::Error::auth_required());
                    }
                    let id = format!("session-{index}");
                    let mut options = catalog();
                    if matches!(initial.scenario, Scenario::ResetMode | Scenario::CyclicReset | Scenario::ChangingCyclicReset | Scenario::RepairRemovesModel) {
                        options.push(select("mode", "Mode", "auto", &["auto", "safe"])
                            .category(SessionConfigOptionCategory::Mode));
                    }
                    if initial.scenario == Scenario::Malformed { options.push(options[0].clone()); }
                    if initial.scenario == Scenario::Empty { options.clear(); }
                    if initial.scenario == Scenario::ChangedType {
                        options[0] = serde_json::from_value(serde_json::json!({
                            "id":"model", "name":"Model", "category":"model",
                            "type":"boolean", "currentValue":true
                        })).unwrap();
                    }
                    created.lock().unwrap().insert(id.clone(), options.clone());
                    let response = NewSessionResponse::new(id).config_options(options);
                    if initial.scenario == Scenario::Empty {
                        responder.respond(response.modes(serde_json::from_value::<SessionModeState>(serde_json::json!({
                            "currentModeId":"safe", "availableModes":[{"id":"safe","name":"Safe"}]
                        })).unwrap()))
                    } else { responder.respond(response) }
                }, agent_client_protocol::on_receive_request!())
            .on_receive_request(
                async move |request: SetSessionConfigOptionRequest, responder: Responder<SetSessionConfigOptionResponse>, _| {
                    let id = request.config_id.to_string();
                    let value = match &request.value {
                        SessionConfigOptionValue::ValueId { value } => value.to_string(),
                        _ => panic!("fixture accepts select values only"),
                    };
                    configure.requests.lock().unwrap().push((id.clone(), value.clone()));
                    if configure.scenario == Scenario::Refuse {
                        return responder.respond_with_error(agent_client_protocol::Error::invalid_params());
                    }
                    let mut all = catalogs.lock().unwrap();
                    let options = all.get_mut(&request.session_id.to_string()).unwrap();
                    if configure.scenario == Scenario::UnconfirmedValue {
                        return responder.respond(SetSessionConfigOptionResponse::new(options.clone()));
                    }
                    if id == "model" && value == "kept" {
                        options[1] = select("thought", "Reasoning effort", "normal", &["normal", "high"])
                            .category(SessionConfigOptionCategory::ThoughtLevel);
                    }
                    if let Some(option) = options.iter_mut().find(|option| option.id.to_string() == id) {
                        if let SessionConfigKind::Select(select) = &mut option.kind { select.current_value = value.into(); }
                    }
                    if configure.scenario == Scenario::Cascade && id == "advanced" {
                        options.retain(|option| option.id.to_string() != "thought");
                    }
                    if matches!(configure.scenario, Scenario::ResetMode | Scenario::CyclicReset | Scenario::ChangingCyclicReset | Scenario::RepairRemovesModel) {
                        if id == "model" {
                            if let SessionConfigKind::Select(mode) = &mut options[3].kind {
                                mode.current_value = "auto".into();
                            }
                        } else if id == "mode" && configure.scenario != Scenario::ResetMode {
                            if configure.scenario == Scenario::RepairRemovesModel {
                                if crate::session_controls::current_value(&options[0]).unwrap() == "kept" {
                                    options.remove(0);
                                }
                            } else if let SessionConfigKind::Select(model) = &mut options[0].kind {
                                model.current_value = "new".into();
                            }
                        }
                        if configure.scenario == Scenario::ChangingCyclicReset {
                            options[0].name = format!("Model response {}", configure.requests.lock().unwrap().len());
                        }
                    }
                    responder.respond(SetSessionConfigOptionResponse::new(options.clone()))
                }, agent_client_protocol::on_receive_request!()))
    }
}
struct Tray;
impl crate::ui::TrayOutput<MockRuntime> for Tray {
    fn apply(
        &self,
        _: &AppHandle<MockRuntime>,
        _: crate::ui::TrayMenuPresentation,
    ) -> Result<(), String> {
        Ok(())
    }
}
struct Harness {
    app: tauri::App<MockRuntime>,
    fixture: Arc<Fixture>,
    candidate: ResolvedAgentRuntime,
    old: ResolvedAgentRuntime,
    root: tempfile::TempDir,
}
impl Harness {
    fn new(scenario: Scenario, defaults: AgentDefaults) -> Self {
        let root = tempfile::tempdir().unwrap();
        let kind = AgentKind::Codex;
        write_selector(
            root.path(),
            kind,
            &Selector {
                current: Some("11111111-1111-4111-8111-111111111111".into()),
                candidate: Some("22222222-2222-4222-8222-222222222222".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let runtime = |id: &str, version: &str| ResolvedAgentRuntime {
            kind,
            adapter_name: "fixture",
            adapter_version: version.into(),
            command: "/never-executed".into(),
            args: vec![],
            installation: Some(acquire_lease(root.path(), kind, id).unwrap()),
        };
        let old = runtime("11111111-1111-4111-8111-111111111111", "1.0.0");
        let candidate = runtime("22222222-2222-4222-8222-222222222222", "2.0.0");
        let fixture = Arc::new(Fixture {
            runtime: Mutex::new(candidate.clone()),
            scenario,
            requests: Mutex::new(vec![]),
            sessions: AtomicUsize::new(0),
            hold_next: AtomicBool::new(false),
            entered: Notify::new(),
            release: Notify::new(),
        });
        let mut state = crate::test_support::state();
        state.store = crate::store::ConfigStore::at_path(root.path().join("settings.json"));
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.config.agent = kind;
            snapshot.config.working_directory = root.path().to_owned();
            snapshot.config.agent_preferences.set(kind, defaults);
            snapshot.agent_selection = AgentSelectionState {
                operation_id: Some(Uuid::new_v4()),
                candidate: Some(kind),
                stage: AgentSelectionStage::Selected,
                config_options: Some(old_catalog()),
                catalog_generation: Some(Uuid::new_v4()),
                catalog_revision: 1,
                ..Default::default()
            };
            snapshot.agent_runtime = AgentRuntimeState {
                operation_id: Some(Uuid::new_v4()),
                agent: Some(kind),
                current_version: Some("1.0.0".into()),
                ..Default::default()
            };
        }
        state.store.save(&state.config().unwrap()).unwrap();
        let app = tauri::test::mock_builder()
            .manage(state)
            .manage(AgentServices(Arc::new(fixture.clone())))
            .manage(crate::ui::TrayPresentation::<MockRuntime>(Arc::new(Tray)))
            .build(crate::product_context())
            .unwrap();
        Self {
            app,
            fixture,
            candidate,
            old,
            root,
        }
    }
    async fn verify(
        &self,
    ) -> Result<VerifiedManagedRuntime, crate::agent::ManagedVerificationError> {
        crate::agent::verify_managed_runtime(self.app.handle(), self.candidate.clone()).await
    }
    fn config_on_disk(&self) -> crate::model::AppConfig {
        crate::model::AppConfig::decode_settings(
            &fs::read(self.root.path().join("settings.json")).unwrap(),
            self.root.path().to_owned(),
        )
        .unwrap()
    }
}
#[tokio::test]
async fn update_catalog_preserves_valid_dependent_override_and_publishes_together() {
    let h = Harness::new(
        Scenario::Normal,
        defaults(&[("thought", "high"), ("model", "kept")]),
    );
    let before = h.app.state::<AppState>().snapshot().unwrap();
    let verified = h.verify().await.unwrap();
    assert!(verified.removed.is_empty());
    assert_eq!(
        h.fixture.requests.lock().unwrap().as_slice(),
        &[
            ("model".into(), "kept".into()),
            ("thought".into(), "high".into())
        ]
    );
    assert!(
        commit_verified_update(h.app.handle(), &h.candidate, verified)
            .unwrap()
            .is_empty()
    );
    let after = h.app.state::<AppState>().snapshot().unwrap();
    assert_eq!(
        after.agent_selection.operation_id,
        before.agent_selection.operation_id
    );
    assert_ne!(
        after.agent_selection.catalog_generation,
        before.agent_selection.catalog_generation
    );
    assert_eq!(after.agent_selection.catalog_model.as_deref(), Some("kept"));
    let published = after.agent_selection.config_options.as_ref().unwrap();
    let models = crate::session_controls::values(&published[0]).unwrap();
    assert!(models.iter().any(|model| model.value.to_string() == "new"));
    assert!(!models
        .iter()
        .any(|model| model.value.to_string() == "removed"));
    assert_eq!(published[0].name, "Model"); // Label changes preserve the exact "kept" override.
    assert_eq!(
        after.agent_runtime.current_version.as_deref(),
        Some("2.0.0")
    );
    assert_eq!(after.config, h.config_on_disk());
    assert_eq!(
        after.config.agent_preferences.codex.tools,
        before.config.agent_preferences.codex.tools
    );
    assert!(
        read_selector(h.root.path(), AgentKind::Codex)
            .unwrap()
            .current
            .as_deref()
            == Some("22222222-2222-4222-8222-222222222222")
    );
    assert_eq!(
        crate::session_controls::current_value(&after.agent_selection.config_options.unwrap()[1])
            .unwrap(),
        "high"
    );
}
#[tokio::test]
async fn removed_model_and_dependent_choice_fall_back_without_configuration_rpc() {
    let h = Harness::new(
        Scenario::Normal,
        defaults(&[("model", "removed"), ("thought", "high")]),
    );
    let verified = h.verify().await.unwrap();
    assert!(verified.defaults.choices.is_empty());
    assert_eq!(verified.removed.len(), 2);
    assert!(h.fixture.requests.lock().unwrap().is_empty());
    let notice = update_defaults_notice(
        &commit_verified_update(h.app.handle(), &h.candidate, verified).unwrap(),
    );
    assert!(notice.contains("Model") && notice.contains("Reasoning effort"));
    assert!(h
        .config_on_disk()
        .agent_preferences
        .codex
        .choices
        .is_empty());
}
#[tokio::test]
async fn supported_mode_reset_by_model_is_reapplied_without_dropping_overrides() {
    let h = Harness::new(
        Scenario::ResetMode,
        defaults(&[("mode", "safe"), ("model", "kept")]),
    );
    let verified = h.verify().await.unwrap();
    assert!(verified.removed.is_empty());
    assert_eq!(
        verified.defaults.choices,
        defaults(&[("mode", "safe"), ("model", "kept")]).choices
    );
    assert_eq!(
        h.fixture.requests.lock().unwrap().as_slice(),
        &[
            ("mode".into(), "safe".into()),
            ("model".into(), "kept".into()),
            ("mode".into(), "safe".into()),
        ]
    );
    assert_eq!(h.fixture.sessions.load(Ordering::SeqCst), 1);
    let options = verified.options.as_ref().unwrap();
    crate::session_controls::confirm_choice(options, "mode", "safe").unwrap();
    crate::session_controls::confirm_choice(options, "model", "kept").unwrap();
    commit_verified_update(h.app.handle(), &h.candidate, verified).unwrap();
    assert_eq!(
        h.config_on_disk().agent_preferences.codex.choices,
        defaults(&[("mode", "safe"), ("model", "kept")]).choices
    );
}

#[tokio::test]
async fn cyclic_and_nonconverging_resets_fail_with_bounded_requests_and_keep_authority() {
    for (scenario, expected_requests) in [
        (Scenario::CyclicReset, 4),
        (Scenario::ChangingCyclicReset, 6),
    ] {
        let h = Harness::new(scenario, defaults(&[("mode", "safe"), ("model", "kept")]));
        let before = h.app.state::<AppState>().snapshot().unwrap();
        let disk = h.config_on_disk();
        let error = h
            .verify()
            .await
            .err()
            .expect("cyclic configuration must fail");
        assert!(
            matches!(error, crate::agent::ManagedVerificationError::Retryable(ref message)
            if message.contains("did not stabilize"))
        );
        assert_eq!(h.fixture.requests.lock().unwrap().len(), expected_requests);
        assert_eq!(h.fixture.sessions.load(Ordering::SeqCst), 1);
        assert_eq!(h.app.state::<AppState>().snapshot().unwrap(), before);
        assert_eq!(h.config_on_disk(), disk);
        assert_eq!(
            read_selector(h.root.path(), AgentKind::Codex)
                .unwrap()
                .current
                .as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
    }
}

#[tokio::test]
async fn removal_during_reapplication_restarts_update_but_strict_validation_fails() {
    let choices = defaults(&[("mode", "safe"), ("model", "kept")]);
    let h = Harness::new(Scenario::RepairRemovesModel, choices.clone());
    let verified = h.verify().await.unwrap();
    assert_eq!(
        verified.defaults.choices,
        defaults(&[("mode", "safe")]).choices
    );
    assert_eq!(verified.removed, defaults(&[("model", "kept")]).choices);
    assert_eq!(h.fixture.sessions.load(Ordering::SeqCst), 2);
    assert_eq!(h.fixture.requests.lock().unwrap().len(), 4);
    let options = verified.options.unwrap();
    crate::session_controls::confirm_choice(&options, "mode", "safe").unwrap();
    crate::session_controls::confirm_choice(&options, "model", "new").unwrap();

    let h = Harness::new(Scenario::RepairRemovesModel, choices.clone());
    let config = h.app.state::<AppState>().config().unwrap();
    assert!(
        crate::agent::validate_agent_defaults(h.app.handle(), &config, &choices)
            .await
            .is_err()
    );
    assert_eq!(h.fixture.sessions.load(Ordering::SeqCst), 1);
    assert_eq!(h.fixture.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn ordinary_defaults_validation_reapplies_supported_resets_but_remains_strict() {
    let choices = defaults(&[("mode", "safe"), ("model", "kept")]);
    let h = Harness::new(Scenario::ResetMode, choices.clone());
    let config = h.app.state::<AppState>().config().unwrap();
    let validated = crate::agent::validate_agent_defaults(h.app.handle(), &config, &choices)
        .await
        .unwrap();
    let options = validated.options.unwrap();
    crate::session_controls::confirm_choice(&options, "mode", "safe").unwrap();
    crate::session_controls::confirm_choice(&options, "model", "kept").unwrap();
    assert_eq!(
        h.fixture.requests.lock().unwrap().as_slice(),
        &[
            ("mode".into(), "safe".into()),
            ("model".into(), "kept".into()),
            ("mode".into(), "safe".into()),
        ]
    );
    for (scenario, defaults, requests) in [
        (Scenario::Normal, defaults(&[("model", "removed")]), 0),
        (
            Scenario::UnconfirmedValue,
            defaults(&[("model", "kept")]),
            1,
        ),
        (Scenario::CyclicReset, choices.clone(), 4),
        (Scenario::ChangingCyclicReset, choices.clone(), 6),
    ] {
        let h = Harness::new(scenario, defaults.clone());
        let before = h.app.state::<AppState>().snapshot().unwrap();
        assert!(
            crate::agent::validate_agent_defaults(h.app.handle(), &before.config, &defaults)
                .await
                .is_err()
        );
        assert_eq!(h.fixture.requests.lock().unwrap().len(), requests);
        assert_eq!(h.app.state::<AppState>().snapshot().unwrap(), before);
        assert_eq!(h.config_on_disk(), before.config);
    }
}

#[tokio::test]
async fn immediate_response_that_does_not_confirm_requested_value_is_not_retried() {
    let h = Harness::new(Scenario::UnconfirmedValue, defaults(&[("model", "kept")]));
    assert!(h.verify().await.is_err());
    assert_eq!(
        h.fixture.requests.lock().unwrap().as_slice(),
        &[("model".into(), "kept".into())]
    );
}

#[tokio::test]
async fn later_catalog_removal_restarts_from_fresh_agent_defaults() {
    let h = Harness::new(
        Scenario::Cascade,
        defaults(&[("thought", "high"), ("model", "kept"), ("advanced", "b")]),
    );
    let verified = h.verify().await.unwrap();
    assert_eq!(h.fixture.sessions.load(Ordering::SeqCst), 2);
    assert_eq!(verified.removed, defaults(&[("thought", "high")]).choices);
    assert!(!verified
        .defaults
        .choices
        .iter()
        .any(|choice| choice.config_id == "thought"));
    assert!(!verified
        .options
        .unwrap()
        .iter()
        .any(|option| option.id.to_string() == "thought"));
}
#[tokio::test]
async fn auth_malformed_catalog_and_advertised_value_refusal_keep_old_authority() {
    for scenario in [Scenario::Auth, Scenario::Malformed, Scenario::Refuse] {
        let h = Harness::new(scenario, defaults(&[("model", "kept")]));
        let before = h.app.state::<AppState>().snapshot().unwrap();
        let disk = h.config_on_disk();
        assert!(h.verify().await.is_err());
        assert_eq!(h.app.state::<AppState>().snapshot().unwrap(), before);
        assert_eq!(h.config_on_disk(), disk);
        let selector = read_selector(h.root.path(), AgentKind::Codex).unwrap();
        assert_eq!(
            selector.current.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );
        assert_eq!(
            selector.candidate.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
    }
}
#[tokio::test]
async fn concurrent_settings_change_rejects_candidate_publication() {
    let h = Harness::new(Scenario::Normal, defaults(&[]));
    let verified = h.verify().await.unwrap();
    h.app
        .state::<AppState>()
        .runtime
        .write()
        .unwrap()
        .agent_selection
        .catalog_revision += 1;
    let before = h.app.state::<AppState>().snapshot().unwrap();
    assert!(commit_verified_update(h.app.handle(), &h.candidate, verified).is_err());
    assert_eq!(h.app.state::<AppState>().snapshot().unwrap(), before);
    assert_eq!(
        read_selector(h.root.path(), AgentKind::Codex)
            .unwrap()
            .current
            .as_deref(),
        Some("11111111-1111-4111-8111-111111111111")
    );
}
#[tokio::test]
async fn stale_preview_from_retired_runtime_cannot_overwrite_updated_catalog() {
    let h = Harness::new(Scenario::Normal, defaults(&[]));
    *h.fixture.runtime.lock().unwrap() = h.old.clone();
    h.fixture.hold_next.store(true, Ordering::SeqCst);
    let before = h.app.state::<AppState>().snapshot().unwrap();
    let preview = crate::commands::preview_agent_model(
        h.app.handle().clone(),
        before.agent_selection.operation_id.unwrap(),
        "model".into(),
        Some("kept".into()),
        before.agent_selection.catalog_generation,
        Some(before.agent_selection.catalog_revision),
    );
    tokio::pin!(preview);
    tokio::select! { _ = h.fixture.entered.notified() => {}, result = &mut preview => panic!("preview completed early: {result:?}") }
    let verified = h.verify().await.unwrap();
    commit_verified_update(h.app.handle(), &h.candidate, verified).unwrap();
    let committed = h.app.state::<AppState>().snapshot().unwrap();
    h.fixture.release.notify_one();
    assert!(preview.await.is_err());
    assert_eq!(h.app.state::<AppState>().snapshot().unwrap(), committed);
}
#[tokio::test]
async fn stale_absent_generation_is_rejected_even_when_revision_matches() {
    let h = Harness::new(Scenario::Normal, defaults(&[]));
    let selected = h.app.state::<AppState>().agent_selection().unwrap();
    assert!(crate::commands::preview_agent_model(
        h.app.handle().clone(),
        selected.operation_id.unwrap(),
        "model".into(),
        None,
        None,
        Some(selected.catalog_revision)
    )
    .await
    .is_err());
    assert!(crate::commands::set_agent_defaults(
        h.app.handle().clone(),
        selected.operation_id.unwrap(),
        defaults(&[]),
        None,
        Some(selected.catalog_revision)
    )
    .await
    .is_err());
    assert_eq!(h.fixture.sessions.load(Ordering::SeqCst), 0);
}
#[test]
fn persistence_failures_never_activate_and_activation_failure_restores_settings() {
    let before = crate::model::AppConfig::new("/fixture".into());
    let mut next = before.clone();
    next.agent_preferences.codex = defaults(&[("model", "kept")]);
    let activated = std::cell::Cell::new(false);
    let error = persist_update(
        &before,
        &next,
        |_| Err("disk full".into()),
        || {
            activated.set(true);
            Ok(())
        },
    )
    .unwrap_err();
    assert!(error.contains("disk full"));
    assert!(!activated.get());
    let written = std::cell::RefCell::new(vec![]);
    let error = persist_update(
        &before,
        &next,
        |config| {
            written.borrow_mut().push(config.clone());
            Ok(())
        },
        || Err("selector failed".into()),
    )
    .unwrap_err();
    assert_eq!(error, "selector failed");
    assert_eq!(*written.borrow(), vec![next.clone(), before.clone()]);
    let calls = std::cell::Cell::new(0);
    let error = persist_update(
        &before,
        &next,
        |_| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err("rollback disk error".into())
            } else {
                Ok(())
            }
        },
        || Err("selector failed".into()),
    )
    .unwrap_err();
    assert!(error.contains("saved defaults may have changed"));
    assert!(error.contains("rollback disk error"));
}
#[tokio::test]
async fn empty_catalog_overrides_legacy_modes_and_changed_control_type_falls_back() {
    let h = Harness::new(
        Scenario::Empty,
        defaults(&[("mode", "safe"), ("model", "kept")]),
    );
    let verified = h.verify().await.unwrap();
    assert_eq!(verified.options, Some(vec![]));
    assert_eq!(verified.agent_default, None);
    assert!(verified.defaults.choices.is_empty());
    assert!(h.fixture.requests.lock().unwrap().is_empty());

    let h = Harness::new(Scenario::ChangedType, defaults(&[("model", "kept")]));
    let verified = h.verify().await.unwrap();
    assert!(verified.defaults.choices.is_empty());
    assert_eq!(verified.removed.len(), 1);
    assert!(h.fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn settings_write_failure_keeps_selector_and_snapshot_before_promotion() {
    let h = Harness::new(Scenario::Normal, defaults(&[("model", "removed")]));
    let verified = h.verify().await.unwrap();
    let before = h.app.state::<AppState>().snapshot().unwrap();
    fs::remove_file(h.root.path().join("settings.json")).unwrap();
    fs::create_dir(h.root.path().join("settings.json")).unwrap();
    assert!(
        commit_verified_update(h.app.handle(), &h.candidate, verified)
            .unwrap_err()
            .contains("Unable to save")
    );
    assert_eq!(h.app.state::<AppState>().snapshot().unwrap(), before);
    assert_eq!(
        read_selector(h.root.path(), AgentKind::Codex)
            .unwrap()
            .current
            .as_deref(),
        Some("11111111-1111-4111-8111-111111111111")
    );
    assert!(!h
        .candidate
        .installation
        .as_ref()
        .unwrap()
        .confirmed
        .load(Ordering::Acquire));
}

#[tokio::test]
async fn stale_save_from_retired_runtime_cannot_change_persisted_defaults() {
    let h = Harness::new(Scenario::Normal, defaults(&[]));
    *h.fixture.runtime.lock().unwrap() = h.old.clone();
    h.fixture.hold_next.store(true, Ordering::SeqCst);
    let before = h.app.state::<AppState>().snapshot().unwrap();
    let save = crate::commands::set_agent_defaults(
        h.app.handle().clone(),
        before.agent_selection.operation_id.unwrap(),
        defaults(&[("model", "kept")]),
        before.agent_selection.catalog_generation,
        Some(before.agent_selection.catalog_revision),
    );
    tokio::pin!(save);
    tokio::select! { _ = h.fixture.entered.notified() => {}, result = &mut save => panic!("save completed early: {result:?}") }
    let verified = h.verify().await.unwrap();
    commit_verified_update(h.app.handle(), &h.candidate, verified).unwrap();
    let committed = h.app.state::<AppState>().snapshot().unwrap();
    let persisted = h.config_on_disk();
    h.fixture.release.notify_one();
    assert!(save.await.is_err());
    assert_eq!(h.app.state::<AppState>().snapshot().unwrap(), committed);
    assert_eq!(h.config_on_disk(), persisted);
}

#[tokio::test]
async fn late_selection_probe_cannot_publish_retired_runtime_catalog() {
    let h = Harness::new(Scenario::Normal, defaults(&[]));
    *h.fixture.runtime.lock().unwrap() = h.old.clone();
    {
        let state = h.app.state::<AppState>();
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.agent_selection.stage = AgentSelectionStage::Unselected;
    }
    h.fixture.hold_next.store(true, Ordering::SeqCst);
    let selection = crate::agent::select_agent(h.app.handle().clone(), AgentKind::Codex);
    tokio::pin!(selection);
    tokio::select! {
        _ = h.fixture.entered.notified() => {},
        result = &mut selection => panic!("selection completed early: {result:?}")
    }
    let verified = h.verify().await.unwrap();
    commit_verified_update(h.app.handle(), &h.candidate, verified).unwrap();
    h.fixture.release.notify_one();
    let outcome = selection.await.unwrap();
    assert_eq!(outcome.stage, AgentSelectionStage::Failed);
    assert!(outcome.config_options.is_none());
    assert!(outcome.catalog_generation.is_none());
    assert!(outcome.error.unwrap().contains("runtime changed"));
}

#[tokio::test]
async fn already_confirmed_exact_candidate_is_idempotent() {
    let h = Harness::new(Scenario::Normal, defaults(&[]));
    let verified = h.verify().await.unwrap();
    confirm_ready(&h.candidate).unwrap();
    let selector = read_selector(h.root.path(), AgentKind::Codex).unwrap();
    commit_verified_update(h.app.handle(), &h.candidate, verified).unwrap();
    assert_eq!(
        read_selector(h.root.path(), AgentKind::Codex).unwrap(),
        selector
    );
}
