//! External ACP profile integration boundaries; live model use is explicitly opt-in.
use super::*;

fn external_id() -> Uuid {
    Uuid::from_u128(1)
}
fn external() -> AgentKind {
    AgentKind::External(external_id())
}
fn profile(command: PathBuf) -> crate::model::ExternalAgentProfile {
    crate::model::ExternalAgentProfile {
        id: external_id(),
        name: "Personal ACP connection".into(),
        command,
        args: vec!["acp".into()],
    }
}

struct TestTray;
impl crate::ui::TrayOutput<tauri::test::MockRuntime> for TestTray {
    fn apply(
        &self,
        _: &tauri::AppHandle<tauri::test::MockRuntime>,
        _: crate::ui::TrayMenuPresentation,
    ) -> Result<(), String> {
        Ok(())
    }
}

fn app(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    crate::configure_shell(
        tauri::test::mock_builder().manage(state),
        crate::platform::Presentation(Arc::new(crate::test_support::UnusedPresentation)),
        crate::ui::TrayPresentation(Arc::new(TestTray)),
        AgentServices(Arc::new(DefaultAgentHost)),
    )
    .build(crate::product_context())
    .unwrap()
}

#[tokio::test]
#[ignore = "Explicit opt-in: sends one synthetic prompt through the installed Goose provider and production HTML publisher"]
async fn installed_goose_persistent_actor_publishes_synthetic_html() {
    installed_external_persistent_actor_publishes_synthetic_html(false).await;
}

#[tokio::test]
#[ignore = "Explicit opt-in: sends one synthetic prompt through installed Copilot and the production HTML publisher"]
// Account-shell environment resolution intentionally does not preserve arbitrary
// test-runner environment overrides. For isolated CLI state, pass a disposable
// executable wrapper that sets COPILOT_HOME and execs the installed Copilot.
async fn installed_copilot_persistent_actor_publishes_synthetic_html() {
    installed_external_persistent_actor_publishes_synthetic_html(true).await;
}

async fn installed_external_persistent_actor_publishes_synthetic_html(copilot: bool) {
    use crate::model::LensOutputBlock;
    use usecase::agent_preferences::{SavedChoice, ToolPolicies, ToolPolicy};

    let executable = std::env::var_os(if copilot {
        "LENS_COPILOT_EXECUTABLE"
    } else {
        "LENS_GOOSE_EXECUTABLE"
    })
    .expect("set the explicitly requested installed Agent executable");
    struct TemporaryDirectory(PathBuf);
    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = TemporaryDirectory(
        std::env::temp_dir().join(format!("lens-goose-synthetic-turn-{}", Uuid::new_v4())),
    );
    std::fs::create_dir_all(&directory.0).unwrap();
    let mut state = crate::test_support::state();
    state.store = crate::store::ConfigStore::at_path(directory.0.join("lens-settings.json"));
    let operation_id = Uuid::new_v4();
    let input = tests::sample_input("Synthetic test fixture: Lens Goose publisher works.");
    let (projection, projection_ref) = tests::sample_projection(&input, &[]);
    {
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.config.agent = external();
        let mut connection = profile(executable.into());
        if copilot {
            connection.args = vec!["--acp".into(), "--stdio".into()];
        }
        snapshot.config.external_agents = vec![connection];
        snapshot.config.working_directory = directory.0.clone();
        snapshot
            .config
            .agent_preferences
            .external
            .entry(external_id())
            .or_default()
            .choices = if copilot {
            vec![]
        } else {
            vec![SavedChoice {
                config_id: "mode".into(),
                value: "approve".into(),
            }]
        };
        snapshot
            .config
            .agent_preferences
            .external
            .entry(external_id())
            .or_default()
            .tools = ToolPolicies {
            read: ToolPolicy::Deny,
            search: ToolPolicy::Deny,
            fetch: ToolPolicy::Deny,
            other: ToolPolicy::Allow,
            ..ToolPolicies::default()
        };
        snapshot.config.agent_prompt_template = AgentPromptTemplate {
            common: "This is a synthetic integration test. {turn_instruction} Use only the Lens HTML publication tool, exactly once, with the current turn_id supplied in the publication metadata. Publish a tiny complete static HTML document containing the exact text LENS_GOOSE_SYNTHETIC_OK. Do not use filesystem, shell, network retrieval, extension-management, or other tools. Do not inspect any files or settings. Do not include scripts or external resources. After successful publication, reply Done and end the turn.".into(),
            full_projection: "The attached observation is synthetic test data.".into(),
            ..AgentPromptTemplate::default()
        };
        snapshot.config.agent_prompt_template.validate().unwrap();
        snapshot.agent_selection = AgentSelectionState {
            candidate: Some(external()),
            stage: AgentSelectionStage::Selected,
            ..Default::default()
        };
        snapshot.lens = LensState {
            operation_id: Some(operation_id),
            stage: LensStage::Ready,
            context: Some(tests::sample_context(1)),
            input: Some(input),
            projection: Some(projection_ref.clone()),
            ..Default::default()
        };
    }
    let config = state.config().unwrap();
    state.store.save(&config).unwrap();
    let app = app(state);
    let mailbox = Arc::new(AgentSessionMailbox::new());
    let (turn, completion) = AgentSessionTurn::new(1, projection_ref.clone(), projection);
    mailbox.replace(turn).unwrap();
    let (shutdown, shutdown_receiver) = watch::channel(false);
    let actor = run_persistent_session_actor(
        app.handle().clone(),
        Uuid::new_v4(),
        AgentSessionIdentity {
            operation_id,
            context_id: Uuid::nil(),
            effective_working_directory: crate::store::effective_working_directory(&config),
            config,
        },
        mailbox,
        shutdown_receiver,
    );
    tokio::pin!(actor);
    let mut actor_finished = false;
    let result = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(120), completion) => {
            result.map_err(|_| "synthetic Goose turn timed out".to_string())
                .and_then(|result| result.map_err(|error| error.to_string()))
                .and_then(|result| result)
        }
        _ = &mut actor => {
            actor_finished = true;
            Err("Goose actor stopped before turn completion".into())
        }
    };
    // Shut down and reap the persistent connection before any assertion can panic.
    let _ = shutdown.send(true);
    if !actor_finished {
        tokio::time::timeout(Duration::from_secs(10), &mut actor)
            .await
            .expect("Goose actor must stop after shutdown");
    }
    result.expect("synthetic Goose turn must complete");
    let lens = app.state::<AppState>().lens().unwrap();
    // Emit only fixed diagnostic labels and counts, never Agent text, arguments,
    // credentials, paths, transport headers, or URLs.
    let controls = lens.session_controls.as_ref();
    let blocks = lens
        .representation
        .as_ref()
        .map(|value| value.output_blocks.as_slice())
        .unwrap_or(&lens.output_blocks);
    let markdown = blocks
        .iter()
        .filter_map(|block| match block {
            LensOutputBlock::Markdown { text, .. } => Some(text.to_lowercase()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!(
        "synthetic external Agent diagnostics: {}",
        serde_json::json!({
            "stage": lens.stage,
            "stop_reason": lens.agent.as_ref().and_then(|agent| agent.stop_reason.as_deref()).map(|reason| match reason { "end_turn" => "end_turn", "cancelled" => "cancelled", "max_tokens" => "max_tokens", _ => "other" }),
            "updates": lens.agent.as_ref().map(|agent| agent.received_updates),
            "html_blocks": blocks.iter().filter(|block| matches!(block, LensOutputBlock::Html { .. })).count(),
            "markdown_characters": markdown.chars().count(),
            "response_mentions": (["done", "publish", "permission", "denied", "cancel", "error", "failed", "cannot", "unable", "lens_goose_synthetic_ok"].into_iter().filter(|word| markdown.contains(word)).collect::<Vec<_>>()),
            "approve_effective": controls.is_some_and(|state| state.effective_mode.as_deref() == Some("approve")),
            "permission_correlation_rejected": controls.and_then(|state| state.notice.as_deref()).is_some_and(|notice| notice.starts_with("Tool permission denied")),
            "control_notice_present": controls.is_some_and(|state| state.notice.is_some()),
            "interaction_statuses": controls.map(|state| state.interactions.iter().map(|interaction| interaction.status).collect::<Vec<_>>()),
        })
    );
    assert_eq!(lens.stage, LensStage::Completed, "{:?}", lens.error);
    let representation = lens.representation.expect("committed representation");
    assert_eq!(representation.projection, projection_ref);
    let html = representation
        .output_blocks
        .iter()
        .filter_map(|block| {
            if let LensOutputBlock::Html {
                text, byte_length, ..
            } = block
            {
                assert_eq!(*byte_length, text.len());
                Some(text)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(html.len(), 1, "one production publisher artifact");
    assert!(html[0].contains("LENS_GOOSE_SYNTHETIC_OK"));
    let controls = lens.session_controls.expect("session controls snapshot");
    assert!(!controls.active, "actor shutdown closes its controls");
}

#[test]
fn changed_executable_or_execution_config_cannot_complete_an_old_probe() {
    let state = crate::test_support::state();
    let operation = Uuid::new_v4();
    {
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.config.agent = AgentKind::Codex;
        snapshot.config.external_agents = vec![profile("/old/agent".into())];
        snapshot.agent_selection = AgentSelectionState {
            operation_id: Some(operation),
            candidate: Some(external()),
            stage: AgentSelectionStage::Checking,
            ..Default::default()
        };
    }
    let expected = state.config().unwrap();
    let app = app(state);
    let state = app.state::<AppState>();
    state.runtime.write().unwrap().config.external_agents[0].command = "/new/agent".into();
    assert!(!complete_agent_selection(app.handle(), operation, external(), &expected).unwrap());
    assert_eq!(
        state.agent_selection().unwrap().stage,
        AgentSelectionStage::Failed
    );
    state.runtime.write().unwrap().config = expected.clone();
    state.runtime.write().unwrap().config.external_agents[0]
        .args
        .push("--new-profile".into());
    assert!(!complete_agent_selection(app.handle(), operation, external(), &expected).unwrap());
    state.runtime.write().unwrap().config = expected.clone();
    state.runtime.write().unwrap().config.external_agents[0].id = Uuid::new_v4();
    assert!(!complete_agent_selection(app.handle(), operation, external(), &expected).unwrap());
    state.runtime.write().unwrap().config = expected.clone();
    state.runtime.write().unwrap().config.working_directory = "/new/directory".into();
    assert!(!complete_agent_selection(app.handle(), operation, external(), &expected).unwrap());
    state.runtime.write().unwrap().config = expected.clone();
    state.runtime.write().unwrap().agent_selection.operation_id = Some(Uuid::new_v4());
    assert!(!complete_agent_selection(app.handle(), operation, external(), &expected).unwrap());
}

#[tokio::test]
async fn unsupported_logout_does_not_invalidate_external_readiness() {
    let state = crate::test_support::state();
    state.runtime.write().unwrap().agent_selection = AgentSelectionState {
        candidate: Some(external()),
        stage: AgentSelectionStage::Selected,
        supports_logout: false,
        ..Default::default()
    };
    let app = app(state);
    assert!(sign_out_selection(app.handle().clone()).await.is_err());
    assert_eq!(
        app.state::<AppState>().agent_selection().unwrap().stage,
        AgentSelectionStage::Selected
    );
}

#[tokio::test]
async fn missing_external_profile_never_installs_a_managed_runtime() {
    let app = app(crate::test_support::state());
    assert!(agent_runtime::resolve_installed(app.handle(), external())
        .await
        .unwrap()
        .is_none());
    assert!(agent_runtime::resolve(app.handle(), external())
        .await
        .unwrap_err()
        .contains("External ACP command"));
    assert!(agent_runtime::resolve_for_session(app.handle(), external())
        .await
        .unwrap_err()
        .contains("External ACP command"));
}

#[tokio::test]
#[ignore = "Explicit opt-in: uses the installed Goose configuration to create an empty ACP session, without model prompts"]
async fn installed_goose_resolver_and_readiness_session() {
    let path = std::env::var_os("LENS_GOOSE_EXECUTABLE")
        .expect("set LENS_GOOSE_EXECUTABLE to an absolute Goose CLI path");
    let state = crate::test_support::state();
    let directory = std::env::temp_dir().join(format!("lens-goose-readiness-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    {
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.config.external_agents = vec![profile(path.into())];
        snapshot.config.agent = external();
        snapshot.config.working_directory = directory.clone();
    }
    let app = app(state);
    let selected = select_agent(app.handle().clone(), external())
        .await
        .unwrap();
    assert_eq!(
        selected.stage,
        AgentSelectionStage::Selected,
        "{:?}",
        selected.error
    );
    assert!(!selected.supports_logout);
    let runtime = agent_runtime::resolve_installed(app.handle(), external())
        .await
        .unwrap()
        .unwrap();
    assert!(runtime.installation.is_none());
    assert_eq!(runtime.args, ["acp"]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn saved_external_profile_survives_failed_acp_verification_without_ready_state() {
    use std::os::unix::fs::PermissionsExt;
    let directory = std::env::temp_dir().join(format!("lens-save-external-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("custom executable");
    std::fs::write(&executable, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let state = crate::test_support::state();
    {
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.config.agent = external();
        snapshot.config.external_agents = vec![profile(executable.clone())];
        snapshot.agent_selection = AgentSelectionState {
            candidate: Some(external()),
            stage: AgentSelectionStage::Selected,
            ..Default::default()
        };
    }
    let app = app(state);
    let mut saved = profile(executable);
    saved.args = vec!["--stdio".into(), "literal space".into()];
    let selection = crate::commands::save_external_agent(
        app.handle().clone(),
        crate::external_agent::ExternalAgentDraft {
            id: saved.id,
            name: saved.name.clone(),
            command: saved.command.to_str().unwrap().into(),
            arguments: shlex::try_join(saved.args.iter().map(String::as_str)).unwrap(),
        },
    )
    .await
    .unwrap();
    assert_eq!(selection.stage, AgentSelectionStage::Failed);
    assert!(!selection.can_select_lens_target());
    assert_eq!(
        app.state::<AppState>().config().unwrap().external_agents,
        vec![saved.clone()]
    );
    assert_eq!(
        app.state::<AppState>().store.load().external_agents,
        vec![saved]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn deleting_unrelated_profile_preserves_selection_and_selected_delete_is_unverified() {
    let mut state = crate::test_support::state();
    let directory = std::env::temp_dir().join(format!("lens-profile-delete-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("user-owned-agent");
    std::fs::write(&executable, "user owned").unwrap();
    state.store = crate::store::ConfigStore::at_path(directory.join("settings.json"));
    let second_id = Uuid::from_u128(2);
    {
        let mut snapshot = state.runtime.write().unwrap();
        let mut second = profile(executable.clone());
        second.id = second_id;
        snapshot.config.external_agents = vec![profile(executable.clone()), second];
        snapshot.config.agent = external();
        snapshot.agent_selection = AgentSelectionState {
            candidate: Some(external()),
            stage: AgentSelectionStage::Selected,
            ..Default::default()
        };
    }
    let application = app(state);
    crate::commands::delete_external_agent(application.handle().clone(), second_id).unwrap();
    assert_eq!(
        application
            .state::<AppState>()
            .snapshot()
            .unwrap()
            .agent_selection
            .selected_agent(),
        Some(external())
    );
    let result =
        crate::commands::delete_external_agent(application.handle().clone(), external_id())
            .unwrap();
    assert_eq!(result.agent, AgentKind::Claude);
    assert!(application
        .state::<AppState>()
        .snapshot()
        .unwrap()
        .agent_selection
        .selected_agent()
        .is_none());
    assert!(executable.exists());
    std::fs::remove_dir_all(directory).unwrap();
}
