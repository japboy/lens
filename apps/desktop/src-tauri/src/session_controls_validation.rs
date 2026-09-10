//! Opt-in debug validation against the production Rust controls and native WKWebView.
//! This module is absent from release builds; it never reads captured user content.
use super::*;
use crate::model::{LensStage, LensState};
use agent_client_protocol::{Client, Responder};

fn check(value: bool, message: &str) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(invalid(message))
    }
}
fn options() -> Vec<SessionConfigOption> {
    vec![SessionConfigOption::select(
        "mode",
        "Mode",
        "safe",
        vec![
            SessionConfigSelectOption::new("safe", "Safe"),
            SessionConfigSelectOption::new("write", "Write"),
        ],
    )
    .category(SessionConfigOptionCategory::Mode)]
}
fn install<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Result<(Arc<SessionControls>, watch::Sender<bool>), Error> {
    close_active(app);
    let operation = Uuid::new_v4();
    app.state::<AppState>()
        .lens_media
        .begin(operation)
        .map_err(|_| invalid("Unable to initialize fixture media ownership"))?;
    crate::app_state::publish_lens_state(
        app,
        LensState {
            operation_id: Some(operation),
            stage: LensStage::Transforming,
            ..LensState::default()
        },
    )
    .map_err(|_| invalid("Unable to publish fixture operation"))?;
    let (shutdown, receiver) = watch::channel(false);
    let (controls, _) = SessionControls::new(
        operation,
        "fixture-session".into(),
        "Validation Agent".into(),
        "safe".into(),
        Some(options()),
        vec![],
        receiver,
    )?;
    controls.begin_turn(Uuid::new_v4())?;
    controls
        .install(
            app,
            &app.state::<AppState>()
                .config()
                .map_err(|_| invalid("Unable to read fixture config"))?,
        )
        .map_err(|_| invalid("Unable to install fixture controls"))?;
    Ok((controls, shutdown))
}
fn ui_response<R: tauri::Runtime>(
    app: &AppHandle<R>,
    marker: &str,
    button: &str,
) -> Result<(), Error> {
    let script = format!(
        r#"(() => {{
      const marker = {marker}; const button = {button}; let attempts = 0;
      const timer = setInterval(async () => {{
        if (++attempts > 200) {{ clearInterval(timer); return; }}
        const view = document.querySelector('lens-overlay-view');
        const root = view?.shadowRoot;
        if (!root) return;
        if (root.querySelector('.overlay-header lens-session-controls,.session-mode-label,.overlay-shell > lens-session-controls')) {{clearInterval(timer); return;}}
        root.querySelector('#diagnostics-tab')?.click(); await view.updateComplete;
        if (root.querySelector('#diagnostics-panel lens-session-controls button')) {{clearInterval(timer);return;}}
        const panel = root.querySelector('.lens-progress-notification') ?? root.querySelector('#lens-progress-notification');
        if (!panel?.textContent.includes(marker)) return;
        if (root.querySelector('.lens-progress-dismiss,.overlay-status-toggle')) {{clearInterval(timer);return;}}
        root.querySelector('#source-tab')?.click(); await view.updateComplete;
        panel.dispatchEvent(new KeyboardEvent('keydown', {{key:'Escape',bubbles:true}}));
        if (!root.querySelector('#source-panel') || !panel.querySelector('.lens-progress-snackbar')) {{clearInterval(timer);return;}}
        const input = panel.querySelector('input[name="answer"]');
        if (input) input.value = 'fixture-answer';
        const target = [...panel.querySelectorAll('button')].find(b => b.textContent.trim() === button);
        if (target && !target.disabled) {{ clearInterval(timer); target.click(); }}
      }}, 50);
    }})()"#,
        marker = serde_json::to_string(marker).unwrap(),
        button = serde_json::to_string(button).unwrap()
    );
    app.get_webview_window(crate::ui::LENS_WINDOW_LABEL)
        .ok_or_else(|| invalid("Missing native validation window"))?
        .eval(&script)
        .map_err(|_| invalid("Unable to evaluate native UI fixture"))
}
fn form() -> CreateElicitationRequest {
    serde_json::from_value(serde_json::json!({"sessionId":"fixture-session","mode":"form","message":"Native form fixture","requestedSchema":{"type":"object","properties":{"answer":{"type":"string","minLength":1}},"required":["answer"]}})).unwrap()
}
fn permission(title: &str) -> RequestPermissionRequest {
    RequestPermissionRequest::new(
        "fixture-session",
        ToolCallUpdate::new("fixture-read", ToolCallUpdateFields::new().title(title)),
        vec![
            PermissionOption::new(
                "fixture-allow",
                "Allow once",
                PermissionOptionKind::AllowOnce,
            ),
            PermissionOption::new(
                "fixture-deny",
                "Reject once",
                PermissionOptionKind::RejectOnce,
            ),
        ],
    )
}
async fn protocol_and_ui<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), Error> {
    let (controls, _shutdown) = install(app)?;
    controls.record_tool(&SessionUpdate::ToolCall(
        ToolCall::new("fixture-read", "Fixture read")
            .kind(ToolKind::Read)
            .raw_input(serde_json::json!({"path":"fixture.txt"})),
    ))?;
    let permission_controls = controls.clone();
    let permission_app = app.clone();
    let elicitation_controls = controls.clone();
    let elicitation_app = app.clone();
    let client = Client
        .builder()
        .on_receive_request(
            async move |request: RequestPermissionRequest,
                        responder: Responder<RequestPermissionResponse>,
                        connection| {
                permission_controls.receive_permission(
                    &permission_app,
                    request,
                    responder,
                    &connection,
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: CreateElicitationRequest,
                        responder: Responder<CreateElicitationResponse>,
                        connection| {
                elicitation_controls.receive_elicitation(
                    &elicitation_app,
                    request,
                    responder,
                    &connection,
                )
            },
            agent_client_protocol::on_receive_request!(),
        );
    Agent.builder().connect_with(client, async |connection: ConnectionTo<Client>| {
        for (button, expected) in [("Allow once", "fixture-allow"), ("Reject once", "fixture-deny"), ("Cancel request", "cancelled")] {
            ui_response(app, "Native permission fixture", button)?;
            let mut request = permission("Native permission fixture");
            if expected == "cancelled" { request.options.retain(|option| option.kind != PermissionOptionKind::RejectOnce); }
            let response = connection.send_request(request).block_task().await?;
            let value = serde_json::to_value(response).unwrap();
            let actual = value["outcome"]["optionId"].as_str().unwrap_or("cancelled");
            check(actual == expected, "Native permission outcome mismatch")?;
            println!("LENS_INTERACTION_CASE=permission_{expected}:passed");
        }
        for (policy, expected) in [(crate::agent_preferences::ToolPolicy::Allow, "fixture-allow"), (crate::agent_preferences::ToolPolicy::Deny, "fixture-deny")] {
            let before = controls.snapshot().unwrap().interactions.len();
            let saved = crate::agent_preferences::AgentDefaults { tools: crate::agent_preferences::ToolPolicies {read:policy, ..Default::default()}, ..Default::default() };
            let restored: crate::agent_preferences::AgentDefaults = serde_json::from_value(serde_json::to_value(saved).unwrap()).unwrap();
            controls.set_initial_authority("safe".into(), restored.tools)?;
            let response = connection.send_request(permission("Automatic permission fixture")).block_task().await?;
            check(serde_json::to_value(response).unwrap()["outcome"]["optionId"] == expected, "Automatic response did not select exact one-shot ID")?;
            check(controls.snapshot().unwrap().interactions.len() == before, "Automatic response opened a dialog")?;
            println!("LENS_INTERACTION_CASE=automatic_{expected}:passed");
        }
        controls.set_initial_authority("safe".into(), Default::default())?;
        for (button, expected) in [("Send response", "accept"), ("Decline", "decline")] {
            ui_response(app, "Native form fixture", button)?;
            let response = connection.send_request(form()).block_task().await?;
            let value = serde_json::to_value(response).unwrap();
            check(value["action"] == expected, "Native form outcome mismatch")?;
            if expected == "accept" { check(value["content"]["answer"] == "fixture-answer", "Native form content mismatch")?; }
            println!("LENS_INTERACTION_CASE=form_{expected}:passed");
        }
        let url: CreateElicitationRequest = serde_json::from_value(serde_json::json!({"sessionId":"fixture-session","mode":"url","message":"Native URL fixture","url":"http://127.0.0.1:9/lens-validation","elicitationId":"fixture-url"})).unwrap();
        ui_response(app, "Native URL fixture", "Decline")?;
        let response = connection.send_request(url).block_task().await?;
        check(serde_json::to_value(response).unwrap()["action"] == "decline", "Native URL decline mismatch")?;
        println!("LENS_INTERACTION_CASE=url_decline:passed");
        let cancelled = connection.send_request(form());
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancelled.cancel()?;
        let result = cancelled.block_task().await?;
        check(serde_json::to_value(result).unwrap()["action"] == "cancel", "SDK cancellation response mismatch")?;
        check(controls.snapshot().unwrap().interactions.last().unwrap().status == InteractionStatus::Cancelled, "SDK cancellation did not revoke pending form")?;
        println!("LENS_INTERACTION_CASE=sdk_request_cancellation:passed");
        let mut wrong = serde_json::to_value(form()).unwrap();
        wrong["sessionId"] = "wrong-session".into();
        let wrong: CreateElicitationRequest = serde_json::from_value(wrong).unwrap();
        check(connection.send_request(wrong).block_task().await.is_err(), "Cross-session elicitation accepted")?;
        println!("LENS_INTERACTION_CASE=wrong_session:passed");
        let first = connection.send_request(form());
        let second = connection.send_request(form());
        tokio::time::sleep(Duration::from_millis(100)).await;
        let snapshot = controls.snapshot().unwrap();
        let pending = snapshot.interactions.iter().filter(|i| i.status == InteractionStatus::Pending).collect::<Vec<_>>();
        check(pending.len() == 2 && pending[0].sequence < pending[1].sequence, "Concurrent requests were blocked or reordered")?;
        check(crate::commands::respond_agent_interaction(app.clone(), Uuid::new_v4(), snapshot.instance_id, pending[0].id, InteractionResponse::Cancel).is_err(), "Cross-operation response accepted")?;
        for (i, text) in [(1, "second"), (0, "first")] {
            controls.respond(app, snapshot.instance_id, pending[i].id, InteractionResponse::Submit {content:serde_json::json!({"answer":text})}).map_err(|_| invalid("Concurrent response failed"))?;
        }
        check(serde_json::to_value(first.block_task().await?).unwrap()["content"]["answer"] == "first", "First responder correlation lost")?;
        check(serde_json::to_value(second.block_task().await?).unwrap()["content"]["answer"] == "second", "Second responder correlation lost")?;
        println!("LENS_INTERACTION_CASE=concurrent_reverse_response_and_wrong_operation:passed");
        let mut approval = serde_json::to_value(form()).unwrap();
        approval["_meta"] = serde_json::json!({"codex_approval_kind":"mcp_tool_call","persist":["session","always"]});
        let approval: CreateElicitationRequest = serde_json::from_value(approval).unwrap();
        check(connection.send_request(approval).block_task().await.is_err(), "Approval extension bypassed tool policy")?;
        println!("LENS_INTERACTION_CASE=elicitation_approval_extension_denied:passed");


        Ok(())
    }).await?;
    check(
        controls
            .snapshot()
            .unwrap()
            .interactions
            .iter()
            .all(|i| i.status != InteractionStatus::Pending && i.details.is_none()),
        "Payload retained after native responses",
    )?;
    controls.close(app);
    Ok(())
}
async fn lifecycle<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), Error> {
    for cause in [
        "turn_end",
        "disconnect",
        "supersession",
        "lens_close",
        "app_shutdown",
    ] {
        let (controls, _shutdown) = install(app)?;
        let lifetime = ControlLifetime {
            app: app.clone(),
            controls: controls.clone(),
        };
        let (id, response) = controls
            .begin_decision(InteractionDetails::Form {
                message: "Cleanup fixture".into(),
                schema: serde_json::to_value(form()).unwrap()["requestedSchema"].clone(),
            })
            .unwrap();
        let old = controls.snapshot().unwrap();
        match cause {
            "turn_end" => controls.end_turn(),
            "disconnect" => drop(lifetime),
            "supersession" => {
                let _replacement = install(app)?;
            }
            "lens_close" => {
                controls
                    .publish(app)
                    .map_err(|_| invalid("Unable to publish close fixture"))?;
                let operation = serde_json::to_string(&old.operation_id).unwrap();
                let script = format!(
                    r#"(() => {{let attempts=0; const timer=setInterval(() => {{
                    const view=document.querySelector('lens-overlay-view');
                    if (++attempts > 100) {{clearInterval(timer); return;}}
                    if (view?.model?.lens?.operation_id !== {operation}) return;
                    const button=view.shadowRoot?.querySelector('button[aria-label="Stop Lens and close"]');
                    if (button && !button.disabled) {{clearInterval(timer); button.click();}}
                }},50);}})()"#
                );
                app.get_webview_window(crate::ui::LENS_WINDOW_LABEL)
                    .ok_or_else(|| invalid("Missing close fixture window"))?
                    .eval(&script)
                    .map_err(|_| invalid("Unable to click native close control"))?;
            }
            "app_shutdown" => close_active(app),
            _ => unreachable!(),
        }
        check(
            matches!(response.await, Ok(InteractionResponse::Cancel)),
            "Cleanup failed to cancel responder",
        )?;
        check(
            controls
                .snapshot()
                .unwrap()
                .interactions
                .iter()
                .all(|i| i.status == InteractionStatus::Cancelled && i.details.is_none()),
            "Cleanup retained pending payload",
        )?;
        check(
            controls
                .respond(
                    app,
                    old.instance_id,
                    id,
                    InteractionResponse::Submit {
                        content: serde_json::json!({"answer":"late"}),
                    },
                )
                .is_err(),
            "Late response accepted",
        )?;
        if cause == "lens_close" {
            for _ in 0..100 {
                if app
                    .get_webview_window(crate::ui::LENS_WINDOW_LABEL)
                    .is_none()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            check(
                app.get_webview_window(crate::ui::LENS_WINDOW_LABEL)
                    .is_none(),
                "Native close command did not destroy its window",
            )?;
        }
        println!("LENS_INTERACTION_CASE={cause}:passed");
    }
    Ok(())
}
async fn high_level_session<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), Error> {
    use agent_client_protocol::SessionMessage;
    let (controls, _shutdown) = install(app)?;
    let client = Client.builder();
    let agent = Agent
        .builder()
        .on_receive_request(
            async |_: NewSessionRequest, responder: Responder<NewSessionResponse>, _connection| {
                responder
                    .respond(NewSessionResponse::new("fixture-session").config_options(options()))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_: PromptRequest,
                   responder: Responder<PromptResponse>,
                   connection: ConnectionTo<Client>| {
                let task_connection = connection.clone();
                connection.spawn(async move {
                    task_connection.send_notification(SessionNotification::new(
                        "fixture-session",
                        SessionUpdate::ToolCall(
                            ToolCall::new("fixture-read", "Ordered permission fixture")
                                .kind(ToolKind::Read)
                                .raw_input(serde_json::json!({"path":"fixture.txt"})),
                        ),
                    ))?;
                    let result = task_connection
                        .send_request(permission("Ordered permission fixture"))
                        .block_task()
                        .await?;
                    check(
                        serde_json::to_value(result).unwrap()["outcome"]["optionId"]
                            == "fixture-allow",
                        "High-level session permission was lost or raced its tool update",
                    )?;
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                })
            },
            agent_client_protocol::on_receive_request!(),
        );
    client
        .connect_with(agent, async |connection| {
            let mut session = connection
                .build_session(std::env::temp_dir())
                .block_task()
                .start_session()
                .await?;
            ui_response(app, "Ordered permission fixture", "Allow once")?;
            session.send_prompt("Run the ordered fixture")?;
            loop {
                match session.read_update().await? {
                    SessionMessage::StopReason(reason) => {
                        check(
                            reason == StopReason::EndTurn,
                            "Unexpected high-level stop reason",
                        )?;
                        break;
                    }
                    SessionMessage::SessionMessage(dispatch) => {
                        crate::agent::record_control_update(
                            app,
                            &controls,
                            &connection,
                            dispatch,
                            None,
                        )
                        .await?;
                    }
                    _ => return Err(invalid("Unsupported high-level session message")),
                }
            }
            Ok(())
        })
        .await?;
    println!("LENS_INTERACTION_CASE=active_session_tool_permission_ordering:passed");
    Ok(())
}

pub async fn run<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), Error> {
    tauri::WebviewWindowBuilder::new(
        app,
        crate::ui::LENS_WINDOW_LABEL,
        tauri::WebviewUrl::App("overlay.html?platform=macos".into()),
    )
    .title("Lens interaction validation")
    .inner_size(720.0, 680.0)
    .build()
    .map_err(|_| invalid("Unable to create native fixture window"))?;
    protocol_and_ui(app).await?;
    high_level_session(app).await?;
    lifecycle(app).await?;
    let (controls, _shutdown) = install(app)?;
    let response = controls
        .decision_with_deadline(
            app,
            InteractionDetails::Form {
                message: "Expiry fixture".into(),
                schema: serde_json::to_value(form()).unwrap()["requestedSchema"].clone(),
            },
            None,
            Duration::from_millis(30),
        )
        .await;
    check(
        matches!(response, InteractionResponse::Cancel),
        "Deadline did not cancel",
    )?;
    check(
        controls
            .snapshot()
            .unwrap()
            .interactions
            .last()
            .unwrap()
            .status
            == InteractionStatus::Expired,
        "Deadline status incorrect",
    )?;
    println!("LENS_INTERACTION_CASE=expiry:passed");

    Ok(())
}
