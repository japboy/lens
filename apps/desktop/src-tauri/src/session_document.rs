//! Ordered, effect-free session content shared by live reception and history replay.
use agent_client_protocol::schema::v1::{
    ContentBlock, EmbeddedResourceResource, SessionUpdate, ToolCallContent, ToolCallStatus,
    ToolCallUpdateFields,
};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_BLOCKS: usize = 16384;
const MAX_HTML: usize = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentBlock {
    Markdown { text: String },
    Image { mime_type: String, data: String },
    Html { text: String },
    Unsupported { content_type: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentEntry {
    Message {
        id: String,
        role: MessageRole,
        blocks: Vec<DocumentBlock>,
    },
    Tool {
        id: String,
        title: String,
        status: ToolCallStatus,
        blocks: Vec<DocumentBlock>,
        accepted_html: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionDocument {
    pub entries: Vec<DocumentEntry>,
    #[serde(skip)]
    message_key: Option<(MessageRole, Option<String>)>,
    #[serde(skip)]
    tools: std::collections::BTreeMap<String, ToolEvidence>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ToolEvidence {
    input: Option<Value>,
    output: Option<Value>,
    explicit_content: bool,
}

impl SessionDocument {
    /// Outgoing prompt messages are recorded once by live ingress, not inferred from output.
    pub fn append_prompt(&mut self, prompt: &[ContentBlock]) -> Result<(), String> {
        self.transaction(|next| {
            next.message_key = None;
            for content in prompt {
                next.append(MessageRole::User, None, content.clone());
            }
        })
    }

    /// Replay and live updates share exactly the same content semantics. No effects are run.
    pub fn record_update(&mut self, update: SessionUpdate) -> Result<(), String> {
        self.transaction(|next| match update {
            SessionUpdate::UserMessageChunk(chunk) => next.append(
                MessageRole::User,
                chunk.message_id.map(|id| id.to_string()),
                chunk.content,
            ),
            SessionUpdate::AgentMessageChunk(chunk) => next.append(
                MessageRole::Assistant,
                chunk.message_id.map(|id| id.to_string()),
                chunk.content,
            ),
            SessionUpdate::ToolCall(call) => {
                let mut fields = ToolCallUpdateFields::new()
                    .title(call.title)
                    .status(call.status)
                    .raw_input(call.raw_input)
                    .raw_output(call.raw_output);
                // Initial calls default to an empty content vector; unlike sparse updates,
                // that default does not explicitly clear a raw MCP result projection.
                if !call.content.is_empty() {
                    fields = fields.content(call.content);
                }
                next.tool(call.tool_call_id.to_string(), fields);
            }
            SessionUpdate::ToolCallUpdate(update) => {
                next.tool(update.tool_call_id.to_string(), update.fields)
            }
            // Thoughts and runtime control updates do not become conversation content.
            _ => {}
        })
    }

    fn transaction(&mut self, apply: impl FnOnce(&mut Self)) -> Result<(), String> {
        let mut next = self.clone();
        apply(&mut next);
        let blocks: usize = next
            .entries
            .iter()
            .map(|entry| match entry {
                DocumentEntry::Message { blocks, .. } | DocumentEntry::Tool { blocks, .. } => {
                    blocks.len()
                }
            })
            .sum();
        let wire = serde_json::to_vec(&next).map_err(|error| error.to_string())?;
        let evidence = serde_json::to_vec(
            &next
                .tools
                .iter()
                .map(|(id, e)| (id, &e.input, &e.output))
                .collect::<Vec<_>>(),
        )
        .map_err(|error| error.to_string())?;
        if next.entries.len() > MAX_ENTRIES
            || blocks > MAX_BLOCKS
            || wire.len().saturating_add(evidence.len()) > MAX_BYTES
        {
            return Err("Session document exceeds bounded history capacity".into());
        }
        *self = next;
        Ok(())
    }

    fn append(&mut self, role: MessageRole, message_id: Option<String>, content: ContentBlock) {
        let key = (role, message_id);
        if self.message_key.as_ref() != Some(&key)
            || !matches!(self.entries.last(), Some(DocumentEntry::Message { role: previous, .. }) if *previous == role)
        {
            self.entries.push(DocumentEntry::Message {
                id: format!("message:{}", self.entries.len()),
                role,
                blocks: vec![],
            });
        }
        self.message_key = Some(key);
        if let Some(DocumentEntry::Message { blocks, .. }) = self.entries.last_mut() {
            let block = convert(content);
            if let (
                Some(DocumentBlock::Markdown { text: previous }),
                DocumentBlock::Markdown { text },
            ) = (blocks.last_mut(), &block)
            {
                previous.push_str(text);
            } else {
                blocks.push(block);
            }
        }
    }

    fn tool(&mut self, id: String, fields: ToolCallUpdateFields) {
        self.message_key = None;
        let entry_id = format!("tool:{id}");
        let index = self
            .entries
            .iter()
            .position(|entry| matches!(entry, DocumentEntry::Tool { id, .. } if *id == entry_id))
            .unwrap_or_else(|| {
                self.entries.push(DocumentEntry::Tool {
                    id: entry_id,
                    title: "Tool".into(),
                    status: ToolCallStatus::Pending,
                    blocks: vec![],
                    accepted_html: None,
                });
                self.entries.len() - 1
            });
        let evidence = self.tools.entry(id).or_default();
        if fields.raw_input.is_some() {
            evidence.input = fields.raw_input;
        }
        if fields.raw_output.is_some() {
            evidence.output = fields.raw_output;
        }
        if let DocumentEntry::Tool {
            title,
            status,
            blocks,
            accepted_html,
            ..
        } = &mut self.entries[index]
        {
            if let Some(value) = fields.title {
                *title = value;
            }
            if let Some(value) = fields.status {
                *status = value;
            }
            if let Some(content) = fields.content {
                evidence.explicit_content = true;
                *blocks = content
                    .into_iter()
                    .map(|item| match item {
                        ToolCallContent::Content(content) => convert(content.content),
                        _ => DocumentBlock::Unsupported {
                            content_type: "tool detail".into(),
                        },
                    })
                    .collect();
            }
            // Codex supplies MCP results in rawOutput instead of ACP tool content.
            // Explicit content (including an update clearing it) remains authoritative.
            // Retained raw output is projected on completion even if it arrived earlier.
            if !evidence.explicit_content {
                blocks.clear();
            }
            if !evidence.explicit_content && *status == ToolCallStatus::Completed {
                if let Some(items) = evidence
                    .output
                    .as_ref()
                    .and_then(successful_tool_result)
                    .and_then(|result| result.get("content"))
                    .and_then(Value::as_array)
                {
                    *blocks = items
                        .iter()
                        .map(|item| {
                            serde_json::from_value::<ContentBlock>(item.clone())
                                .map(convert)
                                .unwrap_or_else(|_| DocumentBlock::Unsupported {
                                    content_type: "tool result content".into(),
                                })
                        })
                        .collect();
                }
            }
            *accepted_html = if *status == ToolCallStatus::Completed {
                accepted_publication(evidence, blocks)
            } else {
                None
            };
        }
    }
}

/// Unwrap the adapter's MCP result envelope without treating errors as success.
fn successful_tool_result(output: &Value) -> Option<&Value> {
    if output.get("isError") == Some(&Value::Bool(true))
        || output.get("error").is_some_and(|error| !error.is_null())
    {
        return None;
    }
    let result = output.get("result").unwrap_or(output);
    if !result.is_object() || result.get("isError") == Some(&Value::Bool(true)) {
        return None;
    }
    Some(result)
}

fn receipt(value: &Value) -> bool {
    value.get("accepted") == Some(&Value::Bool(true))
        && value
            .get("publication_id")
            .and_then(Value::as_str)
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|id| !id.is_nil()))
}

/// A validated receipt means tool acceptance only; it never asserts host publication.
fn accepted_publication(evidence: &ToolEvidence, blocks: &[DocumentBlock]) -> Option<String> {
    let input = evidence.input.as_ref()?;
    let input = input.get("arguments").unwrap_or(input);
    let html = input.get("html")?.as_str()?;
    if html.trim().is_empty()
        || html.len() > MAX_HTML
        || !input
            .get("turn_id")?
            .as_str()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|id| !id.is_nil()))
    {
        return None;
    }
    // A Codex MCP envelope carries both a result and an independent error.
    // Neither a receipt nor a rendered text block can override a failed result.
    let output = match evidence.output.as_ref() {
        Some(output) => Some(successful_tool_result(output)?),
        None => None,
    };
    let accepted = output.is_some_and(|output| {
        receipt(output) || output.get("content").and_then(Value::as_array).is_some_and(|items| items.iter().any(|item| item.get("text").and_then(Value::as_str).and_then(|text| serde_json::from_str::<Value>(text).ok()).is_some_and(|v| receipt(&v))))
    }) || blocks.iter().any(|block| matches!(block, DocumentBlock::Markdown { text } if serde_json::from_str::<Value>(text).is_ok_and(|value| receipt(&value))));
    accepted.then(|| html.to_owned())
}

fn convert(content: ContentBlock) -> DocumentBlock {
    match content {
        ContentBlock::Text(value) => DocumentBlock::Markdown { text: value.text },
        ContentBlock::Image(value)
            if [
                "image/png",
                "image/jpeg",
                "image/webp",
                "image/gif",
                "image/avif",
            ]
            .contains(&value.mime_type.to_ascii_lowercase().as_str())
                && value.data.len() <= 14 * 1024 * 1024
                && BASE64_STANDARD
                    .decode(&value.data)
                    .is_ok_and(|bytes| bytes.len() <= 10 * 1024 * 1024) =>
        {
            DocumentBlock::Image {
                mime_type: value.mime_type.to_ascii_lowercase(),
                data: value.data,
            }
        }
        ContentBlock::Resource(value) => match value.resource {
            EmbeddedResourceResource::TextResourceContents(value)
                if value.mime_type.as_deref().is_some_and(|mime| {
                    mime.split(';')
                        .next()
                        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
                }) && value.text.len() <= MAX_HTML =>
            {
                DocumentBlock::Html { text: value.text }
            }
            EmbeddedResourceResource::TextResourceContents(value) => {
                DocumentBlock::Markdown { text: value.text }
            }
            _ => DocumentBlock::Unsupported {
                content_type: "resource".into(),
            },
        },
        _ => DocumentBlock::Unsupported {
            content_type: "unsupported content".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{ContentChunk, TextContent, ToolCall, ToolCallUpdate};
    fn text(value: &str) -> ContentBlock {
        ContentBlock::Text(TextContent::new(value))
    }
    fn assistant(value: &str) -> SessionUpdate {
        SessionUpdate::AgentMessageChunk(ContentChunk::new(text(value)))
    }
    #[test]
    fn fragmented_and_replayed_text_match_without_deduplication() {
        let mut live = SessionDocument::default();
        live.append_prompt(&[text("ha")]).unwrap();
        live.record_update(assistant("ha")).unwrap();
        live.record_update(assistant("ha")).unwrap();
        let mut replay = SessionDocument::default();
        replay
            .record_update(SessionUpdate::UserMessageChunk(ContentChunk::new(text(
                "ha",
            ))))
            .unwrap();
        replay.record_update(assistant("haha")).unwrap();
        assert_eq!(live, replay);
        assert_eq!(live.entries.len(), 2);
    }
    #[test]
    fn sparse_updates_preserve_position_and_empty_clears() {
        let mut doc = SessionDocument::default();
        doc.record_update(SessionUpdate::ToolCall(
            ToolCall::new("t", "Read").content(vec![text("old").into()]),
        ))
        .unwrap();
        doc.record_update(assistant("after")).unwrap();
        doc.record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t",
            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
        )))
        .unwrap();
        assert!(
            matches!(&doc.entries[0], DocumentEntry::Tool { title, status: ToolCallStatus::Completed, blocks, .. } if title == "Read" && blocks == &vec![DocumentBlock::Markdown { text: "old".into() }])
        );
        doc.record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "t",
            ToolCallUpdateFields::new().content(vec![]),
        )))
        .unwrap();
        assert_eq!(doc.entries.len(), 2);
        assert!(matches!(&doc.entries[0], DocumentEntry::Tool { blocks, .. } if blocks.is_empty()));
    }
    #[test]
    fn rejected_capacity_update_is_atomic() {
        let mut doc = SessionDocument::default();
        for index in 0..MAX_ENTRIES {
            doc.entries.push(DocumentEntry::Message {
                id: index.to_string(),
                role: MessageRole::User,
                blocks: vec![],
            });
        }
        let before = doc.clone();
        assert!(doc.record_update(assistant("overflow")).is_err());
        assert_eq!(doc, before);
    }
    #[test]
    fn serialized_html_keeps_body_but_raw_evidence_stays_private() {
        let mut doc = SessionDocument::default();
        doc.entries.push(DocumentEntry::Message {
            id: "message:0".into(),
            role: MessageRole::Assistant,
            blocks: vec![DocumentBlock::Html {
                text: "<h1>Visible</h1>".into(),
            }],
        });
        doc.tools.insert(
            "private".into(),
            ToolEvidence {
                input: Some(serde_json::json!({"secret_diagnostic":"excluded"})),
                output: None,
                ..Default::default()
            },
        );
        let wire = serde_json::to_string(&doc).unwrap();
        assert!(wire.contains("<h1>Visible</h1>"));
        assert!(!wire.contains("secret_diagnostic"));
    }
    #[test]
    fn thought_chunks_are_not_rendered() {
        let mut doc = SessionDocument::default();
        doc.record_update(SessionUpdate::AgentThoughtChunk(ContentChunk::new(text(
            "private progress",
        ))))
        .unwrap();
        assert!(doc.entries.is_empty());
    }
    #[test]
    fn html_receipt_requires_valid_turn_and_success() {
        let evidence = ToolEvidence {
            input: Some(
                serde_json::json!({"html":"<h1>Hello</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}),
            ),
            output: Some(
                serde_json::json!({"accepted":true,"publication_id":"e9e57747-88ed-47b7-a301-47f6c13e4503"}),
            ),
            ..Default::default()
        };
        assert_eq!(
            accepted_publication(&evidence, &[]).as_deref(),
            Some("<h1>Hello</h1>")
        );
        assert_eq!(
            accepted_publication(
                &ToolEvidence {
                    output: None,
                    ..evidence
                },
                &[]
            ),
            None
        );
    }
    fn codex_result(content: Value) -> Value {
        serde_json::json!({"result":{"content":content},"error":null})
    }

    #[test]
    fn codex_modern_receipt_unwraps_result_without_weakening_turn_or_error_checks() {
        let evidence = ToolEvidence {
            input: Some(
                serde_json::json!({"server":"lens_output","tool":"publish_html","arguments":{
                "html":"<h1>Modern</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}}),
            ),
            output: Some(codex_result(serde_json::json!([{"type":"text","text":
                "{\"accepted\":true,\"publication_id\":\"e9e57747-88ed-47b7-a301-47f6c13e4503\"}"}]))),
            ..Default::default()
        };
        assert_eq!(
            accepted_publication(&evidence, &[]).as_deref(),
            Some("<h1>Modern</h1>")
        );
        for failure in [
            serde_json::json!({"details":"failed"}),
            serde_json::json!(false),
        ] {
            let mut bad = evidence.clone();
            bad.output.as_mut().unwrap()["error"] = failure;
            assert!(accepted_publication(&bad, &[]).is_none());
        }
        let mut bad = evidence.clone();
        bad.output.as_mut().unwrap()["result"]["isError"] = Value::Bool(true);
        assert!(accepted_publication(&bad, &[]).is_none());
        let mut no_turn = evidence;
        no_turn.input.as_mut().unwrap()["arguments"]
            .as_object_mut()
            .unwrap()
            .remove("turn_id");
        assert!(accepted_publication(&no_turn, &[]).is_none());
    }

    #[test]
    fn codex_legacy_embedded_html_is_output_content_not_a_synthetic_receipt() {
        let html = "<h1>Legacy output</h1>";
        let output = codex_result(serde_json::json!([{"type":"resource","resource":{
            "uri":"lens-html://fixture","mimeType":"text/html","text":html}}]));
        let mut document = SessionDocument::default();
        document.record_update(SessionUpdate::ToolCall(ToolCall::new("legacy", "mcp.lens_output.publish_html")
            .status(ToolCallStatus::Completed)
            .raw_input(serde_json::json!({"server":"lens_output","tool":"publish_html","arguments":{"html":html}}))
            .raw_output(output.clone()))).unwrap();
        assert!(
            matches!(&document.entries[0], DocumentEntry::Tool { blocks, accepted_html: None, .. }
            if blocks == &vec![DocumentBlock::Html { text: html.into() }])
        );
        // A later failing result must remove output derived from the old success.
        let mut failed = output;
        failed["result"]["isError"] = Value::Bool(true);
        document
            .record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "legacy",
                ToolCallUpdateFields::new().raw_output(failed),
            )))
            .unwrap();
        assert!(
            matches!(&document.entries[0], DocumentEntry::Tool { blocks, accepted_html: None, .. } if blocks.is_empty())
        );
    }

    #[test]
    fn raw_mcp_results_do_not_replace_explicit_content_and_explicit_empty_clears() {
        let mut document = SessionDocument::default();
        let output = codex_result(serde_json::json!([{"type":"text","text":"raw"}]));
        document
            .record_update(SessionUpdate::ToolCall(
                ToolCall::new("tool", "Tool")
                    .status(ToolCallStatus::Completed)
                    .content(vec![text("explicit").into()])
                    .raw_output(output.clone()),
            ))
            .unwrap();
        assert!(
            matches!(&document.entries[0], DocumentEntry::Tool { blocks, .. }
            if blocks == &vec![DocumentBlock::Markdown { text: "explicit".into() }])
        );
        document
            .record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "tool",
                ToolCallUpdateFields::new()
                    .content(vec![])
                    .raw_output(output.clone()),
            )))
            .unwrap();
        document
            .record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "tool",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .raw_output(output),
            )))
            .unwrap();
        assert!(
            matches!(&document.entries[0], DocumentEntry::Tool { blocks, .. } if blocks.is_empty())
        );
    }
    #[test]
    fn raw_mcp_result_before_completion_is_projected_on_sparse_completion() {
        let mut document = SessionDocument::default();
        document
            .record_update(SessionUpdate::ToolCall(
                ToolCall::new("tool", "Tool")
                    .status(ToolCallStatus::InProgress)
                    .raw_output(codex_result(
                        serde_json::json!([{"type":"text","text":"raw"}]),
                    )),
            ))
            .unwrap();
        assert!(
            matches!(&document.entries[0], DocumentEntry::Tool { blocks, .. } if blocks.is_empty())
        );
        document
            .record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "tool",
                ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
            )))
            .unwrap();
        assert!(
            matches!(&document.entries[0], DocumentEntry::Tool { blocks, .. }
            if blocks == &vec![DocumentBlock::Markdown { text: "raw".into() }])
        );
    }
}
