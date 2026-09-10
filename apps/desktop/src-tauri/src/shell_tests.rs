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
fn html_output_ipc_requires_overlay_and_exact_retained_identity() {
    let state = test_support::state();
    let operation = Uuid::from_u128(501);
    let representation = Uuid::from_u128(502);
    state.runtime.write().unwrap().lens = LensState {
        operation_id: Some(operation),
        representation: Some(LensRepresentation {
            prompt_execution_revision: 1,
            representation_id: representation,
            context_id: Uuid::nil(),
            context_revision: 1,
            projection: crate::live_sync::ProjectionRef::new(
                std::num::NonZeroU64::new(1).unwrap(),
                "0".repeat(64).parse().unwrap(),
            ),
            run_id: Uuid::nil(),
            output_blocks: vec![LensOutputBlock::Html {
                message_id: None,
                resource_id: "html-fixture".into(),
                mime_type: "text/html".into(),
                uri: "urn:fixture".into(),
                byte_length: 14,
                text: "<p>private</p>".into(),
            }],
        }),
        ..LensState::default()
    };
    let app = app(state);
    let settings = window(&app);
    let overlay = tauri::WebviewWindowBuilder::new(&app, "lens-overlay", Default::default())
        .build()
        .unwrap();
    let args = json!({"operationId": operation, "representationId": representation, "resourceId": "html-fixture"});
    assert!(invoke(&settings, "get_html_output", args.clone()).is_err());
    assert_eq!(
        invoke(&overlay, "get_html_output", args.clone()).unwrap(),
        json!("<p>private</p>")
    );
    let snapshot = invoke(&overlay, "get_app_snapshot", json!({})).unwrap();
    assert!(!snapshot.to_string().contains("<p>private</p>"));
    for field in ["operationId", "representationId", "resourceId"] {
        let mut stale = args.clone();
        stale[field] = json!(Uuid::new_v4());
        assert!(invoke(&overlay, "get_html_output", stale).is_err());
    }
    app.state::<AppState>()
        .runtime
        .write()
        .unwrap()
        .lens
        .representation = None;
    assert!(invoke(&overlay, "get_html_output", args).is_err());
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

struct PresetTestTray;
impl<R: tauri::Runtime> crate::ui::TrayOutput<R> for PresetTestTray {
    fn apply(
        &self,
        _: &tauri::AppHandle<R>,
        _: crate::ui::TrayMenuPresentation,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn preset_ipc_round_trip_uses_catalog_authority_and_rejects_stale_edits() {
    let app = crate::configure_shell(
        tauri::test::mock_builder(),
        test_support::state(),
        platform::Presentation(Arc::new(test_support::UnusedPresentation)),
        crate::ui::TrayPresentation(Arc::new(PresetTestTray)),
        crate::agent::AgentServices(Arc::new(test_support::UnusedAgent)),
    )
    .build(crate::product_context())
    .unwrap();
    let settings = window(&app);
    let original = app.state::<AppState>().config().unwrap();
    let template = original.agent_prompt_template.clone();
    let created = invoke(
        &settings,
        "update_prompt_presets",
        json!({"change": {
            "type":"create", "name":"My preset", "template":template
        }}),
    )
    .unwrap();
    let catalog = &created["prompt_presets"];
    assert_eq!(catalog["selected_id"], "visual-learner");
    let preset = catalog["presets"].as_array().unwrap().last().unwrap();
    let id = preset["id"].as_str().unwrap();
    let updated = invoke(
        &settings,
        "update_prompt_presets",
        json!({"change": {
            "type":"update", "id":id, "expected_revision":1, "name":"Renamed", "template":template
        }}),
    )
    .unwrap();
    assert_eq!(updated["prompt_presets"]["execution_revision"], 1);
    assert!(invoke(
        &settings,
        "update_prompt_presets",
        json!({"change": {
            "type":"update", "id":id, "expected_revision":1, "name":"Stale", "template":template
        }})
    )
    .is_err());
    let selected = invoke(
        &settings,
        "update_prompt_presets",
        json!({"change": {"type":"select", "id":id}}),
    )
    .unwrap();
    assert_eq!(selected["prompt_presets"]["selected_id"], id);
    assert_eq!(
        selected["agent_prompt_template"],
        serde_json::to_value(template).unwrap()
    );
    assert_eq!(
        app.state::<AppState>()
            .store
            .load()
            .prompt_presets
            .selected_id,
        id
    );
    let mut bundled_template = original.prompt_presets.presets[0].template.clone();
    bundled_template
        .common
        .push_str("\nPrefer annotated figures.");
    let renamed = invoke(
        &settings,
        "update_prompt_presets",
        json!({"change": {
            "type":"update", "id":"visual-learner", "expected_revision":1,
            "name":"My visual notes", "template":bundled_template
        }}),
    )
    .unwrap();
    let saved = app.state::<AppState>().store.load();
    let visual = saved
        .prompt_presets
        .presets
        .iter()
        .find(|p| p.id == "visual-learner")
        .unwrap();
    assert_eq!(visual.name, "My visual notes");
    assert_eq!(visual.template, bundled_template);
    let deleted = invoke(
        &settings,
        "update_prompt_presets",
        json!({"change": {
            "type":"delete", "id":"visual-learner", "expected_revision":visual.revision
        }}),
    )
    .unwrap();
    assert!(!app
        .state::<AppState>()
        .store
        .load()
        .prompt_presets
        .presets
        .iter()
        .any(|p| p.id == "visual-learner"));
    assert!(invoke(
        &settings,
        "update_prompt_presets",
        json!({"change": {
            "type":"reset_all", "expected_catalog_revision":renamed["prompt_presets"]["revision"]
        }})
    )
    .is_err());
    let restored = invoke(
        &settings,
        "update_prompt_presets",
        json!({"change": {
            "type":"reset_all", "expected_catalog_revision":deleted["prompt_presets"]["revision"]
        }}),
    )
    .unwrap();
    let saved = app.state::<AppState>().store.load();
    assert_eq!(
        serde_json::to_value(&saved.prompt_presets).unwrap(),
        restored["prompt_presets"]
    );
    assert_eq!(saved.prompt_presets.selected_id, "visual-learner");
    assert_eq!(saved.prompt_presets.selected().name, "Visual Learner");
    assert_eq!(saved.prompt_presets.presets.len(), 3);
    let restored_visual = saved
        .prompt_presets
        .presets
        .iter()
        .find(|p| p.id == "visual-learner")
        .unwrap();
    assert_eq!(restored_visual.name, "Visual Learner");
    assert_eq!(
        restored_visual.template,
        original.prompt_presets.presets[0].template
    );
}
