//! Byte-stable migration fixtures captured before splitting the backend workspace.
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
        "window_id":7,"bundle_id":"example.browser","pid":42,
        "title":"Document","application_name":"Browser",
        "frame":{"x":0.0,"y":0.0,"width":100.0,"height":100.0}
    }))
    .unwrap();
    let extraction: ExtractionResult = serde_json::from_value(json!({
        "quality":"full", "resolved_window":null,
        "nodes":[{
            "id":"node-000000","parent_id":null,"order":0,"depth":0,
            "role":"AXWindow","subrole":null,"title":"Document","value":null,
            "description":null,"bounds":{"x":0.0,"y":0.0,"width":100.0,"height":100.0},
            "resource_refs":[],"children":["node-000001"]
        },{
            "id":"node-000001","parent_id":"node-000000","order":1,"depth":1,
            "role":"AXStaticText","subrole":null,"title":null,"value":"A stable observation.",
            "description":null,"bounds":null,"resource_refs":[],"children":[]
        }],
        "text":"", "diagnostics":[]
    }))
    .unwrap();
    let targets = LensTargetSet::try_new(Uuid::nil(), vec![window]).unwrap();
    let context = LensContext::from_captures(
        Uuid::nil(),
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
        // Pinned so the settings UI's copy of the sentinel cannot drift from the Rust one.
        "mode_config_sentinel": usecase::session_controls::MODE_CONFIG_SENTINEL,
    })
}

#[test]
fn serialized_contracts_match_the_pre_workspace_baseline() {
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

#[test]
#[ignore = "Explicit export of production-rendered prompts for live Agent validation"]
fn print_prompt_preset_validation_cases() {
    use crate::live_sync::ProjectionRef;
    use std::num::NonZeroU64;
    use usecase::prompt_presets::bundled_presets;
    let projection = |revision| {
        ProjectionRef::new(
            NonZeroU64::new(revision).unwrap(),
            format!("{revision:064x}").parse().unwrap(),
        )
    };
    let cases: Vec<_> = bundled_presets()
        .into_iter()
        .map(|preset| {
            let turns: Vec<_> = [
                ("initial", AgentPromptMode::FullProjection, projection(1)),
                (
                    "update",
                    AgentPromptMode::SourceCheckpoint {
                        base_projection: projection(1),
                    },
                    projection(2),
                ),
                (
                    "retry",
                    AgentPromptMode::CurrentProjectionRetry {
                        applied_projection: projection(2),
                    },
                    projection(2),
                ),
                (
                    "unchanged",
                    AgentPromptMode::SourceCheckpoint {
                        base_projection: projection(2),
                    },
                    projection(3),
                ),
            ]
            .into_iter()
            .map(|(kind, mode, target)| {
                serde_json::json!({
                    "kind": kind, "prompt": preset.template.render(&mode, &target).unwrap(),
                })
            })
            .collect();
            serde_json::json!({"id": preset.id, "turns": turns})
        })
        .collect();
    println!(
        "PROMPT_PRESET_VALIDATION={}",
        serde_json::to_string(&cases).unwrap()
    );
}
