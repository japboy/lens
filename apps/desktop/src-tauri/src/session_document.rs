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
                let fields = ToolCallUpdateFields::new()
                    .title(call.title)
                    .status(call.status)
                    .content(call.content)
                    .raw_input(call.raw_input)
                    .raw_output(call.raw_output);
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
            *accepted_html = if *status == ToolCallStatus::Completed {
                accepted_publication(evidence, blocks)
            } else {
                None
            };
        }
    }
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
    if evidence
        .output
        .as_ref()
        .is_some_and(|output| output.get("isError") == Some(&Value::Bool(true)))
    {
        return None;
    }
    let accepted = evidence.output.as_ref().is_some_and(|output| {
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
}
