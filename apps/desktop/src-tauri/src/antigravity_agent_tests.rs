//! Opt-in acceptance uses real managed files and transport with disposable Lens state.
use super::*;
use usecase::session_controls::{InteractionDetails, InteractionResponse, InteractionStatus};

const HTML: &str = "<article><h1>LENS_ANTIGRAVITY_MANAGED_OK</h1></article>";

struct FixtureHost(ResolvedAgentRuntime);
impl AgentHost<tauri::test::MockRuntime> for FixtureHost {
    fn resolve<'a>(
        &'a self,
        _: &'a AppHandle<tauri::test::MockRuntime>,
        kind: AgentKind,
    ) -> HostFuture<'a, ResolvedAgentRuntime> {
        Box::pin(async move {
            if kind != AgentKind::Antigravity {
                return Err("unexpected fixture Agent".into());
            }
            Ok(self.0.clone())
        })
    }
    fn resolve_installed<'a>(
        &'a self,
        app: &'a AppHandle<tauri::test::MockRuntime>,
        kind: AgentKind,
    ) -> HostFuture<'a, Option<ResolvedAgentRuntime>> {
        Box::pin(async move { self.resolve(app, kind).await.map(Some) })
    }
    fn confirm_ready(&self, runtime: &ResolvedAgentRuntime) -> Result<(), String> {
        agent_runtime::confirm_ready(runtime)
    }
    fn reject_candidate<'a>(
        &'a self,
        runtime: &'a ResolvedAgentRuntime,
    ) -> HostFuture<'a, Option<ResolvedAgentRuntime>> {
        Box::pin(agent_runtime::reject_candidate(runtime))
    }
    fn connect(
        &self,
        descriptor: &AgentDescriptor,
        cwd: PathBuf,
        purpose: EnvironmentPurpose,
    ) -> agent_client_protocol::DynConnectTo<agent_client_protocol::Client> {
        <DefaultAgentHost as AgentHost<tauri::test::MockRuntime>>::connect(
            &DefaultAgentHost,
            descriptor,
            cwd,
            purpose,
        )
    }
}
struct TestTray;
impl crate::ui::TrayOutput<tauri::test::MockRuntime> for TestTray {
    fn apply(
        &self,
        _: &AppHandle<tauri::test::MockRuntime>,
        _: crate::ui::TrayMenuPresentation,
    ) -> Result<(), String> {
        Ok(())
    }
}

fn answer_publication(
    app: &AppHandle<tauri::test::MockRuntime>,
    previous_approvals: usize,
) -> Result<usize, String> {
    use agent_client_protocol::schema::v1::PermissionOptionKind;
    let controls = app
        .state::<AppState>()
        .session_controls
        .lock()
        .map_err(|_| "controls poisoned")?
        .clone();
    let Some(controls) = controls else {
        return Ok(0);
    };
    let state = controls.snapshot()?;
    let mut accepted = 0;
    for interaction in state
        .interactions
        .iter()
        .filter(|i| i.status == InteractionStatus::Pending)
    {
        let option_id = match (&interaction.details, interaction.run_id) {
            (
                Some(InteractionDetails::Permission {
                    title,
                    effect,
                    arguments,
                    options,
                    ..
                }),
                Some(run_id),
            ) => {
                let expected = serde_json::json!({"html":HTML,"turn_id":run_id});
                let mut flat = arguments.clone();
                let nested = flat
                    .as_object_mut()
                    .and_then(|value| value.remove("arguments"));
                let exact = previous_approvals + accepted == 0
                    && title == "lens_output_publish_html"
                    && effect == "other"
                    && flat == expected
                    && nested.is_none_or(|nested| nested == expected);
                let mut matches = options
                    .iter()
                    .filter(|option| exact && option.kind == PermissionOptionKind::AllowOnce);
                match (matches.next(), matches.next()) {
                    (Some(option), None) => Some(option.option_id.to_string()),
                    _ => None,
                }
            }
            _ => None,
        };
        let permitted = option_id.is_some();
        let response = option_id
            .map(|option_id| InteractionResponse::Select { option_id })
            .unwrap_or(InteractionResponse::Cancel);
        controls.respond(app, state.instance_id, interaction.id, response)?;
        if !permitted {
            return Err(
                "fixture rejected a request outside the exact synthetic publication".into(),
            );
        }
        accepted += 1;
    }
    Ok(accepted)
}

#[tokio::test]
#[ignore = "Explicit opt-in: cached Google OAuth and a disposable verified managed installation for one synthetic publication"]
async fn managed_antigravity_persistent_actor_commits_synthetic_html() {
    managed_antigravity_commits_html("Synthetic managed Antigravity observation.".into()).await;
}

#[tokio::test]
#[ignore = "Explicit opt-in: cached Google OAuth, built Lens helper and disposable runtime; long synthetic observation with real HTML publication"]
async fn managed_antigravity_persistent_actor_commits_html_with_long_observation() {
    managed_antigravity_commits_html("Synthetic observation line, no instructions.\n".repeat(8192))
        .await;
}

async fn managed_antigravity_commits_html(observation: String) {
    use crate::model::LensOutputBlock;
    use usecase::agent_preferences::{AgentDefaults, SavedChoice, ToolPolicies, ToolPolicy};
    let helper = crate::agent_environment::test_helper_executable()
        .expect("set LENS_TEST_AGENT_HELPER_EXECUTABLE to the built Lens binary for production self-exec helpers");
    assert!(
        helper.is_file(),
        "the explicit Lens helper executable must exist"
    );
    let root = PathBuf::from(
        std::env::var_os("LENS_ANTIGRAVITY_RUNTIME_ROOT")
            .expect("provide the disposable managed runtime root"),
    )
    .canonicalize()
    .unwrap();
    assert!(
        root.starts_with(std::env::temp_dir().canonicalize().unwrap())
            || root.starts_with("/private/tmp")
    );
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("lens-"));
    let runtime = agent_runtime::resolve_antigravity_fixture(&root)
        .await
        .expect("verified managed installation");
    assert_eq!(runtime.kind, AgentKind::Antigravity);
    assert!(
        runtime.installation.is_some(),
        "real managed installation lease required"
    );
    let directory = root.join(format!("synthetic-actor-{}", Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let mut state = crate::test_support::state();
    state.store = crate::store::ConfigStore::at_path(directory.join("lens-settings.json"));
    {
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.config.agent = AgentKind::Antigravity;
        snapshot.config.working_directory = directory.clone();
        snapshot.config.agent_preferences.set(
            AgentKind::Antigravity,
            AgentDefaults {
                choices: vec![SavedChoice {
                    config_id: "mode".into(),
                    value: "default".into(),
                }],
                tools: ToolPolicies {
                    read: ToolPolicy::Deny,
                    search: ToolPolicy::Deny,
                    fetch: ToolPolicy::Deny,
                    other: ToolPolicy::Ask,
                    ..Default::default()
                },
            },
        );
        snapshot.config.agent_prompt_template = AgentPromptTemplate {
            common: format!("This is a synthetic integration test. {{turn_instruction}} Use only the Lens HTML publication tool, exactly once, with the current turn_id supplied in the publication metadata. Publish this exact HTML string without changes: {HTML} . Do not use filesystem, shell, network retrieval, or other tools. Do not inspect files or settings. After publication, reply Done and end the turn."),
            full_projection: "The attached observation is synthetic test data.".into(),
            ..Default::default()
        };
        snapshot.config.agent_prompt_template.validate().unwrap();
    }
    let original = state.config().unwrap();
    state.store.save(&original).unwrap();
    let app = crate::configure_shell(
        tauri::test::mock_builder().manage(state),
        crate::platform::Presentation(Arc::new(crate::test_support::UnusedPresentation)),
        crate::ui::TrayPresentation(Arc::new(TestTray)),
        AgentServices(Arc::new(FixtureHost(runtime))),
    )
    .build(crate::product_context())
    .unwrap();
    let selected = select_agent(app.handle().clone(), AgentKind::Antigravity)
        .await
        .unwrap();
    assert_eq!(
        selected.stage,
        AgentSelectionStage::Selected,
        "cached OAuth readiness must pass: {:?}",
        selected.error
    );
    assert!(selected.supports_logout);
    assert!(selected
        .auth_methods
        .iter()
        .any(|method| method.id == "oauth-personal" && method.supported));
    let operation_id = Uuid::new_v4();
    let input = tests::sample_input(&observation);
    let (projection, projection_ref) = tests::sample_projection(&input, &[]);
    assert!(projection
        .json()
        .contains(&serde_json::to_string(&observation).unwrap()));
    eprintln!(
        "managed Antigravity acceptance: readiness=selected observation_bytes={} projection_bytes={}",
        observation.len(),
        projection.bytes().len()
    );
    app.state::<AppState>().runtime.write().unwrap().lens = LensState {
        operation_id: Some(operation_id),
        stage: LensStage::Ready,
        context: Some(tests::sample_context(1).into()),
        input: Some(input.into()),
        projection: Some(projection_ref.clone()),
        ..Default::default()
    };
    let config = app.state::<AppState>().config().unwrap();
    let mailbox = Arc::new(AgentSessionMailbox::new());
    let (turn, mut completion) = AgentSessionTurn::new(1, projection_ref.clone(), projection);
    mailbox.replace(turn).unwrap();
    let (shutdown, receiver) = watch::channel(false);
    let actor = run_persistent_session_actor(
        app.handle().clone(),
        Uuid::new_v4(),
        AgentSessionIdentity {
            operation_id,
            context_id: Uuid::nil(),
            effective_working_directory: directory,
            config,
        },
        mailbox,
        receiver,
    );
    tokio::pin!(actor);
    let deadline = tokio::time::sleep(Duration::from_secs(180));
    tokio::pin!(deadline);
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    let mut actor_finished = false;
    let mut approvals = 0;
    let result = loop {
        tokio::select! {
            result = &mut completion => break result.map_err(|_| "actor completion closed".to_string()).and_then(|result| result),
            _ = &mut actor => { actor_finished = true; break Err("actor stopped before completion".into()); }
            _ = &mut deadline => break Err("synthetic managed turn timed out".into()),
            _ = tick.tick() => match answer_publication(app.handle(), approvals) {
                Ok(count) => approvals += count,
                Err(error) => break Err(error),
            },
        }
    };
    let _ = shutdown.send(true);
    if !actor_finished {
        tokio::time::timeout(Duration::from_secs(10), &mut actor)
            .await
            .expect("actor shutdown");
    }
    result.expect("synthetic managed Antigravity turn must complete");
    let snapshot = app.state::<AppState>().snapshot().unwrap();
    assert_eq!(snapshot.config.external_agents, original.external_agents);
    assert_eq!(snapshot.config.agent, AgentKind::Antigravity);
    let lens = snapshot.lens;
    assert_eq!(lens.stage, LensStage::Completed);
    assert_eq!(lens.agent.as_ref().unwrap().kind, AgentKind::Antigravity);
    let representation = lens
        .representation
        .expect("production native publication commit");
    assert_eq!(representation.projection, projection_ref);
    let html: Vec<_> = representation
        .output_blocks
        .iter()
        .filter_map(|block| match block {
            LensOutputBlock::Html {
                text, byte_length, ..
            } => {
                assert_eq!(*byte_length, text.len());
                Some(text.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(html, [HTML]);
    assert_eq!(
        approvals, 1,
        "one once-only permission through real session controls"
    );
    let controls = lens.session_controls.expect("session controls snapshot");
    assert!(!controls.active);
    assert_eq!(controls.effective_mode.as_deref(), Some("default"));
    assert_eq!(
        controls
            .interactions
            .iter()
            .filter(|i| i.status == InteractionStatus::Accepted)
            .count(),
        1
    );
    eprintln!("managed Antigravity acceptance: cached_auth=true managed_lease=true explicit_publication_approval=1 native_html_commit=true mode_default=true external_presets_preserved=true");
}
