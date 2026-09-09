//! Exact serialized contracts, originally captured before splitting the workspace.
//! Current identity migration replaces native IDs with operation-scoped receipts,
//! records application provenance as facts, and advances dependent schema versions.
//! Projection bytes/digest change only with this deliberate public contract migration.
use crate::lens::{LensContext, LensInput, LensMediaCapture, LensTargetCapture, LensTargetSet};
use crate::live_sync::LensAgentProjection;
use crate::model::{AppConfig, AppSnapshot, ExtractionResult, SelectedWindow};
use crate::prompt_template::{AgentPromptMode, AgentPromptTemplate};
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, ImageContent, SessionUpdate, TextContent, ToolCall, ToolCallStatus,
};
use serde_json::{json, Value};
use uuid::Uuid;

fn contract_snapshot() -> Value {
    let window: SelectedWindow = serde_json::from_value(json!({
        "operation_id":Uuid::from_u128(1),"receipt":Uuid::from_u128(17),"selection_ordinal":1,"application_id":"example.browser",
        "title":"Document","application_name":"Browser",
        "frame":{"x":0.0,"y":0.0,"width":100.0,"height":100.0}
    }))
    .unwrap();
    let read = json!({"target":{"operation_id":Uuid::from_u128(1),"receipt":Uuid::from_u128(17)},"sequence":"18446744073709551615"});
    let extraction: ExtractionResult = serde_json::from_value(json!({
        "geometry":{
            "read":read,
            "window":{"read":read,"frame":{"kind":"macos_desktop_points"},"rect":{"x":0.0,"y":0.0,"width":100.0,"height":100.0}},
            "desktop_to_target":{"read":read,"source":{"kind":"macos_desktop_points"},"destination":{"kind":"target_logical","target":read["target"]},"scale_x":1.0,"scale_y":1.0,"translate_x":0.0,"translate_y":0.0}
        },
        "quality":"full", "resolved_window":null,
        "nodes":[{
            "id":"node-000000","parent_id":null,"order":0,"depth":0,
            "source_api":"macos_ax","semantic_kind":"region","node_purpose":"window_chrome",
            "native_role":"AXWindow","native_subrole":null,"title":"Document","value":null,
            "description":null,"bounds":{"kind":"registered","geometry":{"read":read,"frame":{"kind":"macos_desktop_points"},"rect":{"x":0.0,"y":0.0,"width":100.0,"height":100.0}}},
            "resource_refs":[],"children":["node-000001"]
        },{
            "id":"node-000001","parent_id":"node-000000","order":1,"depth":1,
            "source_api":"macos_ax","semantic_kind":"text","node_purpose":"content",
            "native_role":"AXStaticText","native_subrole":null,"title":null,"value":"A stable observation.",
            "description":null,"bounds":null,"resource_refs":[],"children":[]
        }],
        "text":"", "diagnostics":[]
    }))
    .unwrap();
    let targets = LensTargetSet::try_new(Uuid::from_u128(1), vec![window]).unwrap();
    let context = LensContext::from_captures(
        Uuid::from_u128(1),
        &targets,
        vec![LensTargetCapture {
            accessibility: extraction,
            media: LensMediaCapture::default(),
        }],
    )
    .unwrap();
    let input = LensInput::from_context(&context).unwrap();
    let projection = LensAgentProjection::from_input(&input, &targets, &[]).unwrap();
    let projection_ref = projection.projection_ref(std::num::NonZeroU64::new(1).unwrap());
    let prompt = AgentPromptTemplate::default();
    let config: AppConfig = serde_json::from_value(json!({
        "agent":"codex", "working_directory":"/fixture", "response_prompt":"Explain {this}."
    }))
    .unwrap();
    let mut output = crate::agent_output::AgentOutputCandidate::default();
    for update in [
        SessionUpdate::AgentMessageChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(
                "A stable representation.",
            )))
            .message_id("message-1"),
        ),
        SessionUpdate::ToolCall(
            ToolCall::new("tool-1", "Generated image")
                .status(ToolCallStatus::Completed)
                .content(vec![ContentBlock::Image(
                    ImageContent::new("aW1hZ2U=", "image/png").uri("file:///fixture/generated.png"),
                )
                .into()]),
        ),
    ] {
        output.record_update(update, "read-only").unwrap();
    }
    json!({
        "target_set": targets,
        "context": context,
        "input": input,
        "projection_bytes": String::from_utf8(projection.bytes().to_vec()).unwrap(),
        "projection_digest": projection.digest(),
        "prompt_template": prompt,
        "rendered_full_prompt": prompt.render(&AgentPromptMode::FullProjection, &projection_ref).unwrap(),
        "migrated_config": config,
        "snapshot": AppSnapshot::new(config.clone()),
        "agent_output": output.blocks(),
    })
}

#[test]
fn serialized_contracts_match_the_versioned_workspace_fixture() {
    let actual = contract_snapshot();
    let expected: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/workspace-contracts.json"
    ))
    .unwrap();
    assert_eq!(actual, expected);
}

#[test]
#[ignore = "Explicit fixture capture; never rewrites the checked-in expected result"]
fn print_workspace_contract_baseline() {
    println!(
        "WORKSPACE_CONTRACT_BASELINE={}",
        serde_json::to_string(&contract_snapshot()).unwrap()
    );
}
