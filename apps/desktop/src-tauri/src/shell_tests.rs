//! These invoke the production command registration and state implementation, not duplicated handlers.
use crate::{
    app_state::{self, AppState},
    model::*,
    platform, test_support,
};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc};
use tauri::{test::MockRuntime, Listener, Manager};
use uuid::Uuid;

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

fn window(app: &tauri::App<MockRuntime>) -> tauri::WebviewWindow<MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, "settings", Default::default())
        .build()
        .unwrap()
}

pub(crate) fn invoke(
    window: &tauri::WebviewWindow<MockRuntime>,
    command: &str,
    body: Value,
) -> Result<Value, Value> {
    tauri::test::get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
    )
    .map(|body| body.deserialize().unwrap())
}

#[test]
fn about_ipc_embeds_exact_documents_and_window_is_reused() {
    let app = app(test_support::state());
    let settings = window(&app);
    crate::ui::show_about(app.handle()).unwrap();
    let about = app.get_webview_window("about").unwrap();
    let info = invoke(&about, "get_about_info", json!({})).unwrap();
    assert_eq!(info["name"], "Lens");
    assert_eq!(info["version"], app.package_info().version.to_string());
    assert_eq!(info["copyright"], "Copyright © 2026 Yu Inao");
    assert!(info.get("license").is_none());
    assert!(info.get("notice").is_none());
    let documents = invoke(&about, "get_about_documents", json!({})).unwrap();
    assert_eq!(documents["license"], include_str!("../../../../LICENSE"));
    assert_eq!(documents["notice"], include_str!("../../../../NOTICE"));
    crate::ui::show_about(app.handle()).unwrap();
    assert_eq!(app.webview_windows().len(), 2);
    assert!(app.get_webview_window(settings.label()).is_some());
}

#[test]
fn production_snapshot_ipc_preserves_the_complete_state_and_permission_result() {
    let state = test_support::state();
    let expected = serde_json::to_value(state.snapshot().unwrap()).unwrap();
    let app = app(state);
    let window = window(&app);
    assert_eq!(
        invoke(&window, "get_app_snapshot", json!({})).unwrap(),
        expected
    );
    assert_eq!(
        invoke(&window, "accessibility_permission", json!({})).unwrap(),
        json!(false)
    );
}

#[test]
fn production_snapshot_ipc_reports_poisoned_state_as_an_error() {
    let app = app(test_support::state());
    let handle = app.handle().clone();
    assert!(std::thread::spawn(move || {
        let state = handle.state::<AppState>();
        let _lock = state.runtime.write().unwrap();
        panic!("deliberate fixture lock poisoning");
    })
    .join()
    .is_err());
    let window = window(&app);
    assert_eq!(
        invoke(&window, "get_app_snapshot", json!({})).unwrap_err(),
        json!("application state lock is poisoned")
    );
}

#[test]
fn production_state_update_publishes_exactly_one_complete_revision_and_rejects_stale_work() {
    let state = test_support::state();
    let operation = Uuid::from_u128(1);
    state.runtime.write().unwrap().agent_runtime.operation_id = Some(operation);
    let app = app(state);
    let window = window(&app);
    let (sender, receiver) = mpsc::channel();
    let listener = app.listen_any("app-state-changed", move |event| {
        sender
            .send(serde_json::from_str::<Value>(event.payload()).unwrap())
            .unwrap();
    });
    assert!(
        app_state::update_agent_runtime(app.handle(), operation, |runtime| {
            runtime.stage = AgentRuntimeStage::Downloading;
            runtime.downloaded_bytes = 42;
        })
        .unwrap()
    );
    let event = receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert_eq!(
        event,
        invoke(&window, "get_app_snapshot", json!({})).unwrap()
    );
    assert_eq!(event["revision"], 1);
    assert_eq!(event["agent_runtime"]["downloaded_bytes"], 42);
    assert!(
        !app_state::update_agent_runtime(app.handle(), Uuid::from_u128(2), |_| panic!(
            "stale update ran"
        ))
        .unwrap()
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    app.unlisten(listener);
}

#[test]
fn production_confirmation_ipc_rejects_stale_and_empty_selection_before_effects() {
    let state = test_support::state();
    let operation = Uuid::from_u128(1);
    state.runtime.write().unwrap().lens = LensState {
        operation_id: Some(operation),
        stage: LensStage::Selecting,
        selection: Some(LensTargetSelection {
            selection_id: operation,
            stage: LensTargetSelectionStage::Reviewing,
            maximum_targets: crate::lens::MAX_LENS_TARGETS,
            anchor: None,
            items: vec![],
            notice: None,
        }),
        ..LensState::default()
    };
    let before = state.snapshot().unwrap();
    let app = app(state);
    let window = window(&app);
    assert_eq!(
        invoke(
            &window,
            "confirm_lens_targets",
            json!({"operationId":Uuid::from_u128(2)})
        )
        .unwrap_err(),
        json!(crate::confirm_targets::OPERATION_SUPERSEDED)
    );
    let expected = crate::lens::LensTargetSet::try_new(operation, vec![])
        .unwrap_err()
        .to_string();
    assert_eq!(
        invoke(
            &window,
            "confirm_lens_targets",
            json!({"operationId":operation})
        )
        .unwrap_err(),
        json!(expected)
    );
    assert_eq!(app.state::<AppState>().snapshot().unwrap(), before);
}
