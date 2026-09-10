//! Real confirmation IPC, context/media construction, publication, observer and ACP session.
use crate::{
    agent, agent_runtime::ResolvedAgentRuntime, app_state::AppState, model::*, platform, ui,
};
use agent_client_protocol::{
    schema::{v1::*, ProtocolVersion},
    Agent, Client, ConnectionTo, DynConnectTo, Responder,
};
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    capture::{
        Capture, CaptureBatch, CaptureCoverage, CaptureRequest, CaptureTarget, CapturedImage,
    },
    observation::{
        Observation, ObservationEvents, ObservationRegistration, ObservationRequest,
        ObservationSession, WindowObservationEvent, WindowObservationNotification,
        WindowObservationStart,
    },
    selection::{TargetSelection, WindowPickerReply},
    trust::AccessibilityTrust,
    ExtractionLimits, ImageCaptureLimits, PlatformError, PlatformFuture,
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tauri::{test::MockRuntime, Listener, Manager};
use uuid::Uuid;

const OPERATION: Uuid = Uuid::from_u128(7);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Success,
    HtmlPublication(StopReason),
    NativeUnavailable,
    ObserverUnavailable,
    AgentUnavailable,
    CandidateMissingHttp,
    CandidateAlwaysMissingHttp,
    CandidateAuthRequired,
    CandidateSetupFailure,
    CandidatePromptFailure,
    StopDuringDismiss,
    StopDuringPrompt,
    SwitchDuringPrompt,
    StopDuringExtraction,
}

#[derive(Clone, Debug, PartialEq)]
enum Effect {
    Dismiss,
    Tray,
    Extract,
    Capture,
    Observe,
    Resolve,
    ResolveForSession,
    ConfirmReady(String),
    RejectCandidate(String),
    Connect,
    Initialize,
    NewSession(Value),
    Configure,
    Prompt(Value),
    ObserverClosed,
    Released,
}

struct Fixture {
    scenario: Scenario,
    effects: Mutex<Vec<Effect>>,
    prompt_started: tokio::sync::Notify,
    finish_prompt: tokio::sync::Notify,
    release_started: tokio::sync::Notify,
    finish_release: (Mutex<bool>, std::sync::Condvar),
    extraction_started: tokio::sync::Notify,
    finish_extraction: (Mutex<bool>, std::sync::Condvar),
}

impl Fixture {
    fn record(&self, effect: Effect) {
        self.effects.lock().unwrap().push(effect);
    }
    fn options() -> Vec<SessionConfigOption> {
        vec![SessionConfigOption::select(
            "mode",
            "Mode",
            "safe",
            vec![SessionConfigSelectOption::new("safe", "Safe")],
        )
        .category(SessionConfigOptionCategory::Mode)]
    }
}

impl Accessibility for Fixture {
    fn extract(
        &self,
        target: ExtractionTarget,
        limits: ExtractionLimits,
    ) -> Result<port_platform::model::ExtractionResult, PlatformError> {
        assert!(
            matches!(target, ExtractionTarget::Registered { operation_id: OPERATION, ref identity } if identity.window_id == 7)
        );
        assert_eq!(limits.max_nodes, 30_000);
        self.record(Effect::Extract);
        if self.scenario == Scenario::StopDuringExtraction {
            self.extraction_started.notify_one();
            let (lock, condition) = &self.finish_extraction;
            let (released, timeout) = condition
                .wait_timeout_while(
                    lock.lock().unwrap(),
                    std::time::Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(
                *released && !timeout.timed_out(),
                "controlled extraction was not released"
            );
        }
        if self.scenario == Scenario::NativeUnavailable {
            return Err(PlatformError::Operation("fixture AX unavailable".into()));
        }
        Ok(serde_json::from_value(json!({
            "quality":"full", "resolved_window":{
                "facts":{"title":"Observed document", "application_name":"Fixture",
                    "frame":{"x":10.0,"y":20.0,"width":400.0,"height":300.0}},
                "resolution_score":1.0
            },
            "nodes":[
                {"id":"text","parent_id":null,"order":0,"depth":0,"role":"AXStaticText",
                 "value":"Fixture source text","bounds":null,"children":[]},
                {"id":"image","parent_id":null,"order":1,"depth":0,"role":"AXImage",
                 "description":"Fixture source image", "bounds":{"x":30.0,"y":40.0,"width":32.0,"height":32.0},"children":[]}
            ], "text":"Fixture source text",
            "metrics":{"visited_nodes":2,"text_bytes":19,"resource_ref_count":0,"resource_uri_bytes":0,
                "offscreen_text_nodes":0,"virtualization_signals":0,"truncated_nodes":false,
                "truncated_text":false,"children_read_errors":0,"omitted_resource_refs":0,"resource_read_errors":0},
            "diagnostics":[]
        })).unwrap())
    }
}

impl Capture for Fixture {
    fn capture(
        &self,
        target: CaptureTarget,
        requests: &[CaptureRequest],
        _: ImageCaptureLimits,
    ) -> Result<CaptureBatch, PlatformError> {
        assert_eq!(
            target,
            CaptureTarget::Registered {
                operation_id: OPERATION,
                window_id: 7
            }
        );
        self.record(Effect::Capture);
        if self.scenario == Scenario::NativeUnavailable {
            return Err(PlatformError::Operation(
                "fixture capture unavailable".into(),
            ));
        }
        assert_eq!(requests.len(), 1);
        let bounds = requests[0].bounds.unwrap();
        Ok(CaptureBatch {
            captures: vec![CapturedImage {
                attachment_id: requests[0].id.clone(),
                source_bounds: bounds,
                captured_bounds: bounds,
                coverage: CaptureCoverage::FullRegion,
                pixel_width: 32,
                pixel_height: 32,
                png: include_bytes!("../icons/32x32.png").to_vec(),
            }],
            ..CaptureBatch::default()
        })
    }
}

impl TargetSelection for Fixture {
    fn pick(&self, _: Uuid) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>> {
        panic!("confirmation must reuse the selected target")
    }
    fn release_target(&self, _: Uuid, _: u32) -> Result<(), PlatformError> {
        panic!("confirmation must not replace an individual target")
    }
    fn release_operation(&self, operation: Uuid) -> Result<(), PlatformError> {
        assert_eq!(operation, OPERATION);
        self.record(Effect::Released);
        if self.scenario == Scenario::StopDuringPrompt {
            self.release_started.notify_one();
            let (lock, condition) = &self.finish_release;
            let (released, timeout) = condition
                .wait_timeout_while(
                    lock.lock().unwrap(),
                    std::time::Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(
                *released && !timeout.timed_out(),
                "controlled release was not resumed"
            );
        }
        Ok(())
    }
}
impl AccessibilityTrust for Fixture {
    fn inspect(&self) -> bool {
        true
    }
    fn request(&self) -> bool {
        panic!("permission is already supplied")
    }
}

struct Registration {
    fixture: Arc<Fixture>,
    closed: bool,
}
impl ObservationRegistration for Registration {
    fn close(&mut self) -> Result<(), PlatformError> {
        if !self.closed {
            self.closed = true;
            self.fixture.record(Effect::ObserverClosed);
        }
        Ok(())
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.close().unwrap();
    }
}
struct PendingEvents;
impl ObservationEvents for PendingEvents {
    fn poll_next(&mut self, _: &mut Context<'_>) -> Poll<Option<WindowObservationEvent>> {
        Poll::Pending
    }
}
struct Observer(Arc<Fixture>);
impl Observation for Observer {
    fn observe(&self, request: ObservationRequest) -> Result<ObservationSession, PlatformError> {
        assert_eq!(request.operation_id, OPERATION);
        assert_eq!(request.context_id, OPERATION);
        assert_eq!(request.identity.window_id, 7);
        self.0.record(Effect::Observe);
        if self.0.scenario == Scenario::ObserverUnavailable {
            return Err(PlatformError::Operation(
                "fixture observer unavailable".into(),
            ));
        }
        Ok(ObservationSession {
            registration: Box::new(Registration {
                fixture: self.0.clone(),
                closed: false,
            }),
            events: Box::new(PendingEvents),
            start: WindowObservationStart {
                registered_notifications: vec![WindowObservationNotification::WindowTitleChanged],
                diagnostics: vec![],
            },
        })
    }
}

impl platform::WindowPresentation<MockRuntime> for Fixture {
    fn settings_background(
        &self,
        _: &tauri::AppHandle<MockRuntime>,
    ) -> Result<tauri::utils::config::Color, PlatformError> {
        panic!("unexpected settings background")
    }

    fn present(&self, _: &tauri::WebviewWindow<MockRuntime>) -> Result<(), PlatformError> {
        panic!("the selected preview already exists")
    }
    fn dismiss<'a>(
        &'a self,
        window: &'a tauri::WebviewWindow<MockRuntime>,
    ) -> platform::PresentationFuture<'a> {
        assert_eq!(window.label(), ui::TARGET_SELECTION_WINDOW_LABEL);
        self.record(Effect::Dismiss);
        Box::pin(async move {
            if self.scenario == Scenario::StopDuringDismiss {
                crate::commands::stop_lens(window.app_handle().clone(), OPERATION)
                    .map_err(PlatformError::Operation)?;
            }
            Ok(())
        })
    }
    fn transition<'a>(
        &'a self,
        _: &'a tauri::WebviewWindow<MockRuntime>,
        _: f64,
        _: f64,
        _: f64,
        _: f64,
    ) -> platform::PresentationFuture<'a> {
        panic!("confirmation must not resize its preview")
    }
}
impl ui::TrayOutput<MockRuntime> for Fixture {
    fn apply(
        &self,
        _: &tauri::AppHandle<MockRuntime>,
        _: ui::TrayMenuPresentation,
    ) -> Result<(), String> {
        self.record(Effect::Tray);
        Ok(())
    }
}

struct AgentFixture(Arc<Fixture>);
impl agent::AgentHost<MockRuntime> for AgentFixture {
    fn resolve<'a>(
        &'a self,
        _: &'a tauri::AppHandle<MockRuntime>,
        kind: AgentKind,
    ) -> agent::HostFuture<'a, ResolvedAgentRuntime> {
        self.0.record(Effect::Resolve);
        let scenario = self.0.scenario;
        Box::pin(async move {
            if scenario == Scenario::AgentUnavailable {
                return Err("fixture runtime unavailable".into());
            }
            Ok(ResolvedAgentRuntime {
                kind,
                adapter_name: "fixture-acp",
                adapter_version: "2.0.0".into(),
                installation: None,
                safe_mode_id: "safe",
                command: "/fixture/not-executed".into(),
                args: vec![],
            })
        })
    }
    fn resolve_for_session<'a>(
        &'a self,
        app: &'a tauri::AppHandle<MockRuntime>,
        kind: AgentKind,
    ) -> agent::HostFuture<'a, ResolvedAgentRuntime> {
        self.0.record(Effect::ResolveForSession);
        self.resolve(app, kind)
    }
    fn confirm_ready(&self, runtime: &ResolvedAgentRuntime) -> Result<(), String> {
        self.0
            .record(Effect::ConfirmReady(runtime.adapter_version.clone()));
        Ok(())
    }
    fn reject_candidate<'a>(
        &'a self,
        runtime: &'a ResolvedAgentRuntime,
    ) -> agent::HostFuture<'a, Option<ResolvedAgentRuntime>> {
        self.0
            .record(Effect::RejectCandidate(runtime.adapter_version.clone()));
        Box::pin(async move {
            Ok(Some(ResolvedAgentRuntime {
                adapter_version: "1.0.0".into(),
                ..runtime.clone()
            }))
        })
    }

    fn resolve_installed<'a>(
        &'a self,
        _: &'a tauri::AppHandle<MockRuntime>,
        _: AgentKind,
    ) -> agent::HostFuture<'a, Option<ResolvedAgentRuntime>> {
        panic!("confirmation does not restore an installation")
    }
    fn connect(&self, _: &agent::AgentDescriptor) -> DynConnectTo<Client> {
        let first_connection = !self.0.effects.lock().unwrap().contains(&Effect::Connect);
        self.0.record(Effect::Connect);
        let initialize = self.0.clone();
        let new_session = self.0.clone();
        let configure = self.0.clone();
        let prompt = self.0.clone();
        DynConnectTo::new(
            Agent
                .builder()
                .on_receive_request(
                    async move |request: InitializeRequest,
                                responder: Responder<InitializeResponse>,
                                _| {
                        initialize.record(Effect::Initialize);
                        assert_eq!(request.protocol_version, ProtocolVersion::V1);
                        responder.respond(
                            InitializeResponse::new(ProtocolVersion::V1).agent_capabilities(
                                AgentCapabilities::new()
                                    .mcp_capabilities(McpCapabilities::new().http(
                                        initialize.scenario != Scenario::CandidateAlwaysMissingHttp
                                            && !(first_connection
                                                && initialize.scenario
                                                    == Scenario::CandidateMissingHttp),
                                    ))
                                    .prompt_capabilities(PromptCapabilities::new().image(true)),
                            ),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: NewSessionRequest,
                                responder: Responder<NewSessionResponse>,
                                _| {
                        new_session
                            .record(Effect::NewSession(serde_json::to_value(request).unwrap()));
                        if new_session.scenario == Scenario::CandidateAuthRequired {
                            return responder
                                .respond_with_error(agent_client_protocol::Error::auth_required());
                        }
                        responder.respond(
                            NewSessionResponse::new("fixture-session")
                                .config_options(Fixture::options()),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: SetSessionConfigOptionRequest,
                                responder: Responder<SetSessionConfigOptionResponse>,
                                _| {
                        configure.record(Effect::Configure);
                        if configure.scenario == Scenario::CandidateSetupFailure {
                            return responder.respond_with_error(
                                agent_client_protocol::Error::invalid_params()
                                    .data("fixture saved settings rejected"),
                            );
                        }
                        assert_eq!(request.config_id.to_string(), "mode");
                        assert_eq!(serde_json::to_value(&request).unwrap()["value"], "safe");
                        responder.respond(SetSessionConfigOptionResponse::new(Fixture::options()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |request: PromptRequest,
                                responder: Responder<PromptResponse>,
                                connection: ConnectionTo<Client>| {
                        let prompt_value = serde_json::to_value(&request).unwrap();
                        prompt.record(Effect::Prompt(prompt_value.clone()));
                        if prompt.scenario == Scenario::CandidatePromptFailure {
                            return responder.respond_with_error(
                                agent_client_protocol::Error::internal_error()
                                    .data("fixture provider prompt failure"),
                            );
                        }
                        if matches!(prompt.scenario, Scenario::HtmlPublication(_))
                            || prompt.scenario == Scenario::StopDuringPrompt
                        {
                            publish_fixture_html(&prompt, &prompt_value).await;
                        }
                        let first_switch_prompt = prompt.scenario == Scenario::SwitchDuringPrompt
                            && prompt
                                .effects
                                .lock()
                                .unwrap()
                                .iter()
                                .filter(|effect| matches!(effect, Effect::Prompt(_)))
                                .count()
                                == 1;
                        if prompt.scenario == Scenario::StopDuringPrompt || first_switch_prompt {
                            let fixture = prompt.clone();
                            let sender = connection.clone();
                            return connection.spawn(async move {
                                fixture.prompt_started.notify_one();
                                fixture.finish_prompt.notified().await;
                                sender.send_notification(SessionNotification::new(
                                    request.session_id,
                                    SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                        ContentBlock::Text(TextContent::new(
                                            "Late cancelled interpretation",
                                        )),
                                    )),
                                ))?;
                                responder.respond(PromptResponse::new(StopReason::EndTurn))
                            });
                        }
                        if let Scenario::HtmlPublication(stop_reason) = prompt.scenario {
                            // HTML-only turns must not depend on a text chunk for completion.
                            return responder.respond(PromptResponse::new(stop_reason));
                        }
                        connection.send_notification(SessionNotification::new(
                            request.session_id,
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new("Fixture interpretation")),
                            )),
                        ))?;
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    },
                    agent_client_protocol::on_receive_request!(),
                ),
        )
    }
}

/// Exercise the actual registered MCP endpoint instead of fabricating ACP HTML updates.
async fn publish_fixture_html(fixture: &Fixture, prompt: &Value) {
    let server = fixture
        .effects
        .lock()
        .unwrap()
        .iter()
        .find_map(|effect| {
            if let Effect::NewSession(request) = effect {
                Some(request["mcpServers"][0].clone())
            } else {
                None
            }
        })
        .unwrap();
    let context: Value = prompt["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|block| {
            let text = block["text"]
                .as_str()
                .or_else(|| block["resource"]["text"].as_str())?;
            let value: Value = serde_json::from_str(text).ok()?;
            (value["kind"] == "lens_output_publication").then_some(value)
        })
        .expect("explicit publication context");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let post = |payload: Value, session: Option<&str>| {
        let mut request = client
            .post(server["url"].as_str().unwrap())
            .header(
                "authorization",
                server["headers"][0]["value"].as_str().unwrap(),
            )
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("mcp-protocol-version", "2025-03-26")
            .body(payload.to_string());
        if let Some(session) = session {
            request = request.header("mcp-session-id", session);
        }
        request.send()
    };
    let response = post(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}
    }}), None).await.unwrap();
    assert!(response.status().is_success());
    let session = response
        .headers()
        .get("mcp-session-id")
        .map(|v| v.to_str().unwrap().to_owned());
    let _ = response.text().await.unwrap();
    assert!(post(
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        session.as_deref()
    )
    .await
    .unwrap()
    .status()
    .is_success());
    let response = post(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"publish_html","arguments":{"turn_id":context["turn_id"],"html":"<h1>Fixture HTML</h1>"}
    }}), session.as_deref()).await.unwrap();
    assert!(response.status().is_success());
    let raw = response.text().await.unwrap();
    let result: Value = serde_json::from_str(&raw).unwrap_or_else(|_| {
        raw.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .find_map(|data| serde_json::from_str(data).ok())
            .expect("MCP response")
    });
    assert!(result.get("error").is_none());
    assert_ne!(result["result"]["isError"], true);
}

struct Harness {
    app: tauri::App<MockRuntime>,
    window: tauri::WebviewWindow<MockRuntime>,
    fixture: Arc<Fixture>,
    snapshots: Arc<Mutex<Vec<AppSnapshot>>>,
    listener: tauri::EventId,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.app.unlisten(self.listener);
        if self
            .app
            .state::<AppState>()
            .lens()
            .is_ok_and(|lens| lens.operation_id == Some(OPERATION))
        {
            let _ = crate::commands::stop_lens(self.app.handle().clone(), OPERATION);
        }
    }
}

fn setup(scenario: Scenario) -> Harness {
    let fixture = Arc::new(Fixture {
        scenario,
        effects: Mutex::new(vec![]),
        prompt_started: tokio::sync::Notify::new(),
        finish_prompt: tokio::sync::Notify::new(),
        release_started: tokio::sync::Notify::new(),
        finish_release: (Mutex::new(false), std::sync::Condvar::new()),
        extraction_started: tokio::sync::Notify::new(),
        finish_extraction: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let services = platform::Services {
        selection: fixture.clone(),
        accessibility: fixture.clone(),
        capture: fixture.clone(),
        observation: Arc::new(Observer(fixture.clone())),
        trust: fixture.clone(),
    };
    let state = AppState::with_config(
        services,
        crate::store::ConfigStore::at_path(
            std::env::temp_dir().join(format!("lens-unused-settings-{}", Uuid::new_v4())),
        ),
        AppConfig::new("/fixture".into()),
    );
    state.lens_media.begin(OPERATION).unwrap();
    let selected: SelectedWindow = serde_json::from_value(json!({"window_id":7,"bundle_id":"example.fixture","pid":42,
        "title":"Selected document","application_name":"Fixture","frame":{"x":10.0,"y":20.0,"width":400.0,"height":300.0}})).unwrap();
    {
        let mut snapshot = state.runtime.write().unwrap();
        snapshot.config.agent = AgentKind::Codex;
        snapshot.agent_selection.stage = AgentSelectionStage::Selected;
        snapshot.agent_selection.candidate = Some(AgentKind::Codex);
        snapshot.lens = LensState {
            operation_id: Some(OPERATION),
            stage: LensStage::Selecting,
            selection: Some(LensTargetSelection {
                selection_id: OPERATION,
                stage: LensTargetSelectionStage::Reviewing,
                maximum_targets: crate::lens::MAX_LENS_TARGETS,
                anchor: Some(selected.facts.frame),
                items: vec![LensTargetSelectionItem {
                    id: "macos:example.fixture:7".into(),
                    window: selected,
                    preview_uri: None,
                    preview_error: None,
                }],
                notice: None,
            }),
            ..LensState::default()
        };
    }
    let app = crate::configure_shell(
        tauri::test::mock_builder(),
        state,
        platform::Presentation(fixture.clone()),
        ui::TrayPresentation(fixture.clone()),
        agent::AgentServices(Arc::new(AgentFixture(fixture.clone()))),
    )
    .build(crate::product_context())
    .unwrap();
    let window = tauri::WebviewWindowBuilder::new(&app, "settings", Default::default())
        .build()
        .unwrap();
    tauri::WebviewWindowBuilder::new(&app, ui::TARGET_SELECTION_WINDOW_LABEL, Default::default())
        .build()
        .unwrap();
    let snapshots = Arc::new(Mutex::new(Vec::new()));
    let recorded = snapshots.clone();
    let handle = app.handle().clone();
    let checked_initial_media = std::sync::atomic::AtomicBool::new(false);
    let listener = app.listen_any("app-state-changed", move |event| {
        let snapshot: AppSnapshot = serde_json::from_str(event.payload()).unwrap();
        if let Some(context) = &snapshot.lens.context {
            if context.revision == 1
                && snapshot.lens.stage == LensStage::Ready
                && !checked_initial_media.swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                let payloads = handle
                    .state::<AppState>()
                    .lens_media
                    .payloads_for_context(OPERATION, context.revision)
                    .unwrap();
                assert_eq!(
                    context.media.len(),
                    payloads.len(),
                    "state and media must publish atomically"
                );
            }
        }
        recorded.lock().unwrap().push(snapshot);
    });
    Harness {
        app,
        window,
        fixture,
        snapshots,
        listener,
    }
}

fn invoke(window: &tauri::WebviewWindow<MockRuntime>, command: &str) -> LensState {
    serde_json::from_value(
        crate::shell_tests::invoke(window, command, json!({"operationId":OPERATION})).unwrap(),
    )
    .unwrap()
}

#[test]
fn confirmation_ipc_runs_real_context_publication_observer_and_acp_session() {
    let harness = setup(Scenario::Success);
    let Harness {
        ref app,
        ref window,
        ref fixture,
        ref snapshots,
        ..
    } = harness;
    let result = invoke(window, "confirm_lens_targets");
    assert_eq!(result.stage, LensStage::Completed);
    assert!(result.representation.is_some());
    let representation = result.representation.as_ref().unwrap();
    assert_eq!(representation.context_id, OPERATION);
    assert_eq!(representation.context_revision, 1);
    assert_eq!(
        representation.projection,
        result.projection.clone().unwrap()
    );
    let output = serde_json::to_string(&representation.output_blocks).unwrap();
    assert!(output.contains("Fixture interpretation"));
    let effects = fixture.effects.lock().unwrap().clone();
    let position = |effect: &Effect| effects.iter().position(|actual| actual == effect).unwrap();
    assert!(position(&Effect::Dismiss) < position(&Effect::Extract));
    assert!(position(&Effect::Extract) < position(&Effect::Capture));
    assert!(position(&Effect::Capture) < position(&Effect::Observe));
    assert!(position(&Effect::Observe) < position(&Effect::Resolve));
    assert!(position(&Effect::Initialize) < position(&Effect::Configure));
    let request = effects
        .iter()
        .find_map(|e| {
            if let Effect::NewSession(v) = e {
                Some(v)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(request["cwd"], "/fixture");
    assert_eq!(request["mcpServers"].as_array().unwrap().len(), 1);
    let server = &request["mcpServers"][0];
    assert_eq!(server["name"], "lens_output");
    assert_eq!(server["type"], "http");
    let endpoint = reqwest::Url::parse(server["url"].as_str().unwrap()).unwrap();
    assert_eq!(endpoint.scheme(), "http");
    assert_eq!(endpoint.host_str(), Some("127.0.0.1"));
    assert!(endpoint.port().is_some());
    assert_eq!(server["headers"][0]["name"], "Authorization");
    assert!(server["headers"][0]["value"]
        .as_str()
        .unwrap()
        .starts_with("Bearer "));
    assert!(server.get("command").is_none());
    let prompt = effects
        .iter()
        .find_map(|e| {
            if let Effect::Prompt(v) = e {
                Some(v)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(prompt["sessionId"], "fixture-session");
    assert!(prompt["prompt"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["type"] == "image"));
    assert!(serde_json::to_string(prompt)
        .unwrap()
        .contains("Fixture source text"));
    // MockRuntime queues destruction; this harness does not run its native event loop.
    assert_eq!(effects.iter().filter(|e| **e == Effect::Dismiss).count(), 1);
    assert!(app.get_webview_window(ui::LENS_WINDOW_LABEL).is_some());
    let snapshots = snapshots.lock().unwrap();
    assert!(snapshots
        .iter()
        .any(|s| s.lens.stage == LensStage::Extracting));
    assert!(snapshots
        .iter()
        .any(|s| s.lens.stage == LensStage::Ready && s.lens.context.is_some()));
    assert!(snapshots
        .iter()
        .any(|s| s.lens.stage == LensStage::Completed));
    drop(snapshots);
    invoke(window, "stop_lens");
    assert!(fixture
        .effects
        .lock()
        .unwrap()
        .contains(&Effect::ObserverClosed));
    assert!(fixture.effects.lock().unwrap().contains(&Effect::Released));
}

#[test]
fn http_publication_commits_exact_private_html_for_all_non_cancelled_stops() {
    for stop_reason in [
        StopReason::EndTurn,
        StopReason::MaxTokens,
        StopReason::MaxTurnRequests,
        StopReason::Refusal,
    ] {
        let harness = setup(Scenario::HtmlPublication(stop_reason));
        let first = invoke(&harness.window, "confirm_lens_targets");
        assert_eq!(first.stage, LensStage::Completed, "{stop_reason:?}");
        let retained = harness.app.state::<AppState>().lens().unwrap();
        let html: Vec<_> = retained
            .representation
            .as_ref()
            .unwrap()
            .output_blocks
            .iter()
            .filter_map(|block| match block {
                LensOutputBlock::Html { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(html, vec!["<h1>Fixture HTML</h1>"], "{stop_reason:?}");
    }
}

#[test]
fn cancelled_agent_response_discards_published_html() {
    let harness = setup(Scenario::HtmlPublication(StopReason::Cancelled));
    invoke(&harness.window, "confirm_lens_targets");
    let retained = harness.app.state::<AppState>().lens().unwrap();
    assert!(retained.representation.is_none());
    assert!(!retained
        .output_blocks
        .iter()
        .any(|block| matches!(block, LensOutputBlock::Html { .. })));
}

#[test]
fn incompatible_candidate_falls_back_once_before_exactly_one_prompt() {
    let harness = setup(Scenario::CandidateMissingHttp);
    let result = invoke(&harness.window, "confirm_lens_targets");
    assert_eq!(result.stage, LensStage::Completed);
    let effects = harness.fixture.effects.lock().unwrap().clone();
    assert_eq!(
        effects
            .iter()
            .filter(|e| **e == Effect::ResolveForSession)
            .count(),
        1
    );
    assert_eq!(effects.iter().filter(|e| **e == Effect::Connect).count(), 2);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::RejectCandidate(_)))
            .count(),
        1
    );
    assert!(effects.contains(&Effect::RejectCandidate("2.0.0".into())));
    assert!(effects.contains(&Effect::ConfirmReady("1.0.0".into())));
    assert!(!effects.contains(&Effect::ConfirmReady("2.0.0".into())));
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Prompt(_)))
            .count(),
        1
    );
    let configured = effects
        .iter()
        .position(|e| *e == Effect::Configure)
        .unwrap();
    let confirmed = effects
        .iter()
        .position(|e| matches!(e, Effect::ConfirmReady(_)))
        .unwrap();
    let prompted = effects
        .iter()
        .position(|e| matches!(e, Effect::Prompt(_)))
        .unwrap();
    assert!(configured < confirmed && confirmed < prompted);
    invoke(&harness.window, "stop_lens");
}

#[test]
fn incompatible_fallback_is_not_retried_or_prompted_again() {
    let harness = setup(Scenario::CandidateAlwaysMissingHttp);
    let result = invoke(&harness.window, "confirm_lens_targets");
    assert_eq!(result.stage, LensStage::Failed);
    let effects = harness.fixture.effects.lock().unwrap().clone();
    assert_eq!(effects.iter().filter(|e| **e == Effect::Connect).count(), 2);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::RejectCandidate(_)))
            .count(),
        1
    );
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::ConfirmReady(_) | Effect::Prompt(_))));
    invoke(&harness.window, "stop_lens");
}

#[test]
fn authentication_or_saved_settings_failure_never_rolls_back_candidate() {
    for (scenario, stage) in [
        (
            Scenario::CandidateAuthRequired,
            LensStage::AuthenticationRequired,
        ),
        (Scenario::CandidateSetupFailure, LensStage::Failed),
    ] {
        let harness = setup(scenario);
        let result = invoke(&harness.window, "confirm_lens_targets");
        assert_eq!(result.stage, stage);
        let effects = harness.fixture.effects.lock().unwrap().clone();
        assert_eq!(effects.iter().filter(|e| **e == Effect::Connect).count(), 1);
        assert!(!effects.iter().any(|e| matches!(
            e,
            Effect::RejectCandidate(_) | Effect::ConfirmReady(_) | Effect::Prompt(_)
        )));
        invoke(&harness.window, "stop_lens");
    }
}

#[test]
fn provider_failure_after_confirmation_never_rolls_back_or_replays_prompt() {
    let harness = setup(Scenario::CandidatePromptFailure);
    let result = invoke(&harness.window, "confirm_lens_targets");
    assert_eq!(result.stage, LensStage::Failed);
    let effects = harness.fixture.effects.lock().unwrap().clone();
    assert_eq!(effects.iter().filter(|e| **e == Effect::Connect).count(), 1);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Prompt(_)))
            .count(),
        1
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::ConfirmReady(_)))
            .count(),
        1
    );
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::RejectCandidate(_))));
    let configured = effects
        .iter()
        .position(|e| *e == Effect::Configure)
        .unwrap();
    let confirmed = effects
        .iter()
        .position(|e| matches!(e, Effect::ConfirmReady(_)))
        .unwrap();
    let prompted = effects
        .iter()
        .position(|e| matches!(e, Effect::Prompt(_)))
        .unwrap();
    assert!(configured < confirmed && confirmed < prompted);
    invoke(&harness.window, "stop_lens");
}

#[test]
fn unavailable_source_publishes_failure_without_observation_or_agent_work() {
    let harness = setup(Scenario::NativeUnavailable);
    let Harness {
        app: ref _app,
        ref window,
        ref fixture,
        ref snapshots,
        ..
    } = harness;
    let result = invoke(window, "confirm_lens_targets");
    assert_eq!(result.stage, LensStage::Failed);
    assert!(result.input.is_none());
    assert!(result.error.as_ref().unwrap().contains("fixture"));
    assert!(!fixture
        .effects
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, Effect::Observe | Effect::Resolve | Effect::Connect)));
    assert!(snapshots
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.lens.stage == LensStage::Failed));
    invoke(window, "stop_lens");
}

#[test]
fn unavailable_observer_degrades_health_but_keeps_real_agent_transformation() {
    let harness = setup(Scenario::ObserverUnavailable);
    let Harness {
        app: ref _app,
        ref window,
        ref fixture,
        ..
    } = harness;
    let result = invoke(window, "confirm_lens_targets");
    assert_eq!(result.stage, LensStage::Completed);
    let live = result.live.unwrap();
    assert_eq!(live.health, LensSourceHealth::Unavailable);
    assert_eq!(live.freshness, LensFreshness::Unverified);
    assert!(fixture
        .effects
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, Effect::Prompt(_))));
    invoke(window, "stop_lens");
}

#[test]
fn unavailable_agent_runtime_publishes_failure_without_opening_transport() {
    let harness = setup(Scenario::AgentUnavailable);
    let Harness {
        app: ref _app,
        ref window,
        ref fixture,
        ref snapshots,
        ..
    } = harness;
    let result = invoke(window, "confirm_lens_targets");
    assert_eq!(result.stage, LensStage::Failed);
    assert!(result
        .error
        .as_ref()
        .unwrap()
        .contains("fixture runtime unavailable"));
    assert!(snapshots
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.lens.stage == LensStage::Ready));
    assert!(!fixture.effects.lock().unwrap().contains(&Effect::Connect));
    invoke(window, "stop_lens");
}

#[test]
fn stop_during_preview_dismissal_rejects_confirmation_before_source_work() {
    let harness = setup(Scenario::StopDuringDismiss);
    let Harness {
        ref app,
        ref window,
        ref fixture,
        ..
    } = harness;
    let error = crate::shell_tests::invoke(
        window,
        "confirm_lens_targets",
        json!({"operationId":OPERATION}),
    )
    .unwrap_err();
    assert_eq!(error, json!(crate::confirm_targets::OPERATION_SUPERSEDED));
    assert_eq!(
        app.state::<AppState>().lens().unwrap(),
        LensState::default()
    );
    let effects = fixture.effects.lock().unwrap();
    assert!(effects.contains(&Effect::Dismiss));
    assert!(effects.contains(&Effect::Released));
    assert!(!effects.iter().any(|e| matches!(
        e,
        Effect::Extract | Effect::Capture | Effect::Observe | Effect::Resolve
    )));
}

#[test]
fn stop_during_agent_prompt_revokes_publication_and_releases_observation() {
    let harness = setup(Scenario::StopDuringPrompt);
    let Harness {
        ref app,
        ref window,
        ref fixture,
        ref snapshots,
        ..
    } = harness;
    let invocation_window = window.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let invocation = std::thread::spawn(move || {
        sender
            .send(crate::shell_tests::invoke(
                &invocation_window,
                "confirm_lens_targets",
                json!({"operationId":OPERATION}),
            ))
            .unwrap();
    });
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.prompt_started.notified(),
        )
        .await
        .unwrap();
    });
    let stopping_window = window.clone();
    let stopping = std::thread::spawn(move || invoke(&stopping_window, "stop_lens"));
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            fixture.release_started.notified(),
        )
        .await
        .unwrap();
    });
    // Hold Stop after cancellation but before state clearance. The concurrent IPC
    // must finish in this interval, rather than racing the final state publication.
    fixture.finish_prompt.notify_one();
    let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
    let (lock, condition) = &fixture.finish_release;
    *lock.lock().unwrap() = true;
    condition.notify_one();
    assert_eq!(stopping.join().unwrap(), LensState::default());
    invocation.join().unwrap();
    // An acknowledged turn can return the stopped snapshot; a closed actor mailbox
    // reports cancellation. Neither response may contain a late interpretation.
    if let Ok(result) = result.unwrap() {
        let lens = serde_json::from_value::<LensState>(result).unwrap();
        assert_eq!(lens.operation_id, Some(OPERATION));
        assert_eq!(
            lens.live.unwrap().lifecycle,
            LensMonitoringLifecycle::Stopped
        );
        assert_ne!(lens.stage, LensStage::Completed);
        assert!(lens.output_blocks.is_empty());
        assert!(lens.representation.is_none());
        assert!(lens.pending_representation.is_none());
    }
    assert_eq!(
        app.state::<AppState>().lens().unwrap(),
        LensState::default()
    );
    assert!(!snapshots
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.lens.stage == LensStage::Completed || s.lens.representation.is_some()));
    let effects = fixture.effects.lock().unwrap();
    assert!(effects.contains(&Effect::ObserverClosed));
    assert!(effects.contains(&Effect::Released));
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::Prompt(_)))
            .count(),
        1
    );
    assert!(!effects
        .iter()
        .any(|e| matches!(e, Effect::RejectCandidate(_))));
}

#[test]
fn stop_during_extraction_does_not_start_capture_or_publish_stale_context() {
    let harness = setup(Scenario::StopDuringExtraction);
    let invocation_window = harness.window.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let invocation = std::thread::spawn(move || {
        sender
            .send(crate::shell_tests::invoke(
                &invocation_window,
                "confirm_lens_targets",
                json!({"operationId":OPERATION}),
            ))
            .unwrap();
    });
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            harness.fixture.extraction_started.notified(),
        )
        .await
        .unwrap();
    });
    invoke(&harness.window, "stop_lens");
    let (lock, condition) = &harness.fixture.finish_extraction;
    *lock.lock().unwrap() = true;
    condition.notify_one();
    let result = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    invocation.join().unwrap();
    assert!(result.is_err());
    assert_eq!(
        harness.app.state::<AppState>().lens().unwrap(),
        LensState::default()
    );
    assert!(!harness
        .snapshots
        .lock()
        .unwrap()
        .iter()
        .any(|s| s.lens.context.is_some()));
    assert!(!harness
        .fixture
        .effects
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, Effect::Capture | Effect::Observe | Effect::Resolve)));
}

#[test]
fn preset_switch_starts_fresh_session_for_same_sources_and_rejects_late_old_output() {
    let harness = setup(Scenario::SwitchDuringPrompt);
    let invocation_window = harness.window.clone();
    let invocation = std::thread::spawn(move || {
        crate::shell_tests::invoke(
            &invocation_window,
            "confirm_lens_targets",
            json!({"operationId":OPERATION}),
        )
    });
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            harness.fixture.prompt_started.notified(),
        )
        .await
        .unwrap();
    });
    let before = harness.app.state::<AppState>().lens().unwrap();
    let config = crate::commands::update_prompt_presets(
        usecase::prompt_presets::PromptPresetMutation::Select {
            id: "conceptual-learner".into(),
        },
        harness.app.handle().clone(),
    )
    .unwrap();
    tauri::async_runtime::block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let lens = harness.app.state::<AppState>().lens().unwrap();
                if lens.representation.as_ref().is_some_and(|result| {
                    result.prompt_execution_revision == config.prompt_presets.execution_revision
                }) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    });
    let completed = harness.app.state::<AppState>().lens().unwrap();
    assert_eq!(completed.operation_id, before.operation_id);
    assert_eq!(
        completed.target_set.as_ref().unwrap().selection_id,
        before.target_set.as_ref().unwrap().selection_id
    );
    assert_eq!(
        completed
            .target_set
            .as_ref()
            .unwrap()
            .targets
            .iter()
            .map(|target| &target.identity)
            .collect::<Vec<_>>(),
        before
            .target_set
            .as_ref()
            .unwrap()
            .targets
            .iter()
            .map(|target| &target.identity)
            .collect::<Vec<_>>()
    );
    assert_eq!(completed.projection, before.projection);
    harness.fixture.finish_prompt.notify_one();
    let _ = invocation.join().unwrap();
    let after = harness.app.state::<AppState>().lens().unwrap();
    assert_eq!(after.representation, completed.representation);
    assert!(after.representation.as_ref().unwrap().output_blocks.iter().any(|block|
        matches!(block, crate::model::LensOutputBlock::Markdown { text, .. } if text == "Fixture interpretation")));
    let effects = harness.fixture.effects.lock().unwrap();
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::Observe))
            .count(),
        1
    );
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::NewSession(_)))
            .count(),
        2
    );
    let prompts = effects
        .iter()
        .filter_map(|effect| {
            if let Effect::Prompt(value) = effect {
                Some(value)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(prompts.len(), 2);
    assert!(serde_json::to_string(prompts[1])
        .unwrap()
        .contains("Lead with the central idea"));
}
