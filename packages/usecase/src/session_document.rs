//! Ordered, effect-free session content shared by live reception and history replay.
use crate::live_sync::{LensDeliveryCoverage, LensDeliveryMode};
use agent_client_protocol_schema::v1::{
    ContentBlock, EmbeddedResourceResource, SessionUpdate, ToolCallContent, ToolCallStatus,
    ToolCallUpdateFields,
};
use base64::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

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

/// Native rendering permission derived only from validated publication evidence.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HtmlMode {
    #[default]
    Static,
    Interactive,
}
impl HtmlMode {
    fn is_static(&self) -> bool {
        *self == Self::Static
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentEntry {
    Message {
        id: String,
        role: MessageRole,
        blocks: Vec<DocumentBlock>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery: Option<LensDeliveryCoverage>,
    },
    Tool {
        id: String,
        title: String,
        status: ToolCallStatus,
        blocks: Vec<DocumentBlock>,
        accepted_html: Option<String>,
        #[serde(default, skip_serializing_if = "HtmlMode::is_static")]
        accepted_html_mode: HtmlMode,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionDocument {
    pub entries: Vec<DocumentEntry>,
    #[serde(skip)]
    message_key: Option<(MessageRole, Option<String>)>,
    #[serde(skip)]
    tools: std::collections::BTreeMap<String, ToolEvidence>,
    #[serde(skip)]
    accounting: Accounting,
}

/// Derived cache; excluded from both the wire document and semantic equality.
#[derive(Debug, Clone, Default)]
struct Accounting {
    initialized: bool,
    last_changed_entry: Option<usize>,
    bytes: usize,
    blocks: usize,
    entries: usize,
    tool_indices: std::collections::HashMap<String, usize>,
}
impl PartialEq for Accounting {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
fn wire_size(value: &impl Serialize) -> usize {
    // All document/evidence values are concrete JSON-compatible types.
    serde_json::to_vec(value)
        .expect("document values serialize")
        .len()
}
fn block_count(entry: &DocumentEntry) -> usize {
    match entry {
        DocumentEntry::Message { blocks, .. } | DocumentEntry::Tool { blocks, .. } => blocks.len(),
    }
}
fn capacity(bytes: usize, entries: usize, blocks: usize) -> Result<(), String> {
    if bytes > MAX_BYTES || entries > MAX_ENTRIES || blocks > MAX_BLOCKS {
        Err("Session document exceeds bounded history capacity".into())
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ToolEvidence {
    input: Option<Value>,
    output: Option<Value>,
    explicit_content: bool,
    tool_name: Option<String>,
}

impl SessionDocument {
    /// Public entry changed by the most recent reducer operation, if any.
    pub fn last_changed_entry(&self) -> Option<usize> {
        self.accounting.last_changed_entry
    }

    /// Outgoing prompt messages are recorded once by live ingress, not inferred from output.
    pub fn append_prompt(&mut self, prompt: &[ContentBlock]) -> Result<(), String> {
        self.accounting.last_changed_entry = None;
        self.ensure_accounting();
        let mut candidate = Self::default();
        for content in prompt {
            candidate.append_unchecked(MessageRole::User, None, content.clone());
        }
        if let Some(mut entry) = candidate.entries.pop() {
            if let DocumentEntry::Message { id, .. } = &mut entry {
                *id = format!("message:{}", self.entries.len());
            }
            let bytes =
                self.accounting.bytes + wire_size(&entry) + usize::from(!self.entries.is_empty());
            let blocks = self.accounting.blocks + block_count(&entry);
            capacity(bytes, self.entries.len() + 1, blocks)?;
            self.entries.push(entry);
            self.accounting.last_changed_entry = Some(self.entries.len() - 1);
            self.accounting.bytes = bytes;
            self.accounting.blocks = blocks;
            self.accounting.entries = self.entries.len();
            self.message_key = Some((MessageRole::User, None));
        } else {
            capacity(
                self.accounting.bytes,
                self.entries.len(),
                self.accounting.blocks,
            )?;
            self.message_key = None;
        }
        Ok(())
    }

    /// Replay and live updates share exactly the same content semantics. No effects are run.
    pub fn record_update(&mut self, update: SessionUpdate) -> Result<(), String> {
        self.accounting.last_changed_entry = None;
        match update {
            SessionUpdate::UserMessageChunk(chunk) => self.append(
                MessageRole::User,
                chunk.message_id.map(|id| id.to_string()),
                chunk.content,
            ),
            SessionUpdate::AgentMessageChunk(chunk) => self.append(
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
                self.tool(call.tool_call_id.to_string(), fields)
            }
            SessionUpdate::ToolCallUpdate(update) => {
                self.tool(update.tool_call_id.to_string(), update.fields)
            }
            // Thoughts and runtime control updates do not become conversation content.
            _ => Ok(()),
        }
    }

    fn ensure_accounting(&mut self) {
        if self.accounting.initialized && self.accounting.entries == self.entries.len() {
            return;
        }
        self.accounting = Accounting {
            initialized: true,
            last_changed_entry: None,
            bytes: wire_size(self)
                + wire_size(
                    &self
                        .tools
                        .iter()
                        .map(|(id, e)| (id, &e.input, &e.output))
                        .collect::<Vec<_>>(),
                ),
            blocks: self.entries.iter().map(block_count).sum(),
            entries: self.entries.len(),
            tool_indices: self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| match entry {
                    DocumentEntry::Tool { id, .. } => {
                        id.strip_prefix("tool:").map(|id| (id.to_owned(), index))
                    }
                    _ => None,
                })
                .collect(),
        };
    }

    fn append(
        &mut self,
        role: MessageRole,
        message_id: Option<String>,
        content: ContentBlock,
    ) -> Result<(), String> {
        self.ensure_accounting();
        let key = (role, message_id);
        let incoming_delivery = (role == MessageRole::User)
            .then(|| delivery_coverage(&content))
            .flatten();
        let block = convert(content);
        let same = self.message_key.as_ref() == Some(&key)
            && matches!(self.entries.last(), Some(DocumentEntry::Message { role: previous, .. }) if *previous == role);
        if same {
            let Some(DocumentEntry::Message {
                blocks, delivery, ..
            }) = self.entries.last_mut()
            else {
                unreachable!()
            };
            let merge = matches!(
                (blocks.last(), &block),
                (
                    Some(DocumentBlock::Markdown { .. }),
                    DocumentBlock::Markdown { .. }
                )
            );
            let added_bytes = if merge {
                let DocumentBlock::Markdown { text } = &block else {
                    unreachable!()
                };
                wire_size(text) - 2
            } else {
                wire_size(&block) + usize::from(!blocks.is_empty())
            };
            let delivery_bytes = if delivery.is_none() {
                incoming_delivery
                    .as_ref()
                    .map_or(0, |value| wire_size(value) + ",\"delivery\":".len())
            } else {
                0
            };
            let bytes = self.accounting.bytes + added_bytes + delivery_bytes;
            let count = self.accounting.blocks + usize::from(!merge);
            capacity(bytes, self.accounting.entries, count)?;
            if merge {
                let (
                    Some(DocumentBlock::Markdown { text: previous }),
                    DocumentBlock::Markdown { text },
                ) = (blocks.last_mut(), block)
                else {
                    unreachable!()
                };
                previous.push_str(&text);
            } else {
                blocks.push(block);
            }
            if delivery.is_none() {
                *delivery = incoming_delivery;
            }
            self.accounting.bytes = bytes;
            self.accounting.blocks = count;
            if added_bytes + delivery_bytes > 0 {
                self.accounting.last_changed_entry = Some(self.entries.len() - 1);
            }
        } else {
            let entry = DocumentEntry::Message {
                id: format!("message:{}", self.entries.len()),
                role,
                blocks: vec![block],
                delivery: incoming_delivery,
            };
            let bytes =
                self.accounting.bytes + wire_size(&entry) + usize::from(!self.entries.is_empty());
            capacity(bytes, self.entries.len() + 1, self.accounting.blocks + 1)?;
            self.entries.push(entry);
            self.accounting.last_changed_entry = Some(self.entries.len() - 1);
            self.accounting.bytes = bytes;
            self.accounting.blocks += 1;
            self.accounting.entries = self.entries.len();
        }
        self.message_key = Some(key);
        Ok(())
    }

    fn tool(&mut self, id: String, fields: ToolCallUpdateFields) -> Result<(), String> {
        self.ensure_accounting();
        let index = self.accounting.tool_indices.get(&id).copied();
        let mut candidate = Self::default();
        if let Some(index) = index {
            candidate.entries.push(self.entries[index].clone());
        }
        if let Some(evidence) = self.tools.get(&id) {
            candidate.tools.insert(id.clone(), evidence.clone());
        }
        candidate.tool_unchecked(id.clone(), fields);
        let entry = candidate.entries.pop().expect("tool entry");
        let evidence = candidate.tools.remove(&id).expect("tool evidence");
        let old_entry_bytes = index.map_or(0, |i| wire_size(&self.entries[i]));
        let old_blocks = index.map_or(0, |i| block_count(&self.entries[i]));
        let old_evidence = self.tools.get(&id);
        let old_evidence_bytes = old_evidence.map_or(0, |e| wire_size(&(&id, &e.input, &e.output)));
        let bytes = self.accounting.bytes - old_entry_bytes - old_evidence_bytes
            + wire_size(&entry)
            + wire_size(&(&id, &evidence.input, &evidence.output))
            + usize::from(index.is_none() && !self.entries.is_empty())
            + usize::from(old_evidence.is_none() && !self.tools.is_empty());
        let blocks = self.accounting.blocks - old_blocks + block_count(&entry);
        capacity(
            bytes,
            self.entries.len() + usize::from(index.is_none()),
            blocks,
        )?;
        if let Some(index) = index {
            if self.entries[index] != entry {
                self.accounting.last_changed_entry = Some(index);
            }
            self.entries[index] = entry;
        } else {
            self.accounting
                .tool_indices
                .insert(id.clone(), self.entries.len());
            self.entries.push(entry);
            self.accounting.last_changed_entry = Some(self.entries.len() - 1);
        }
        self.tools.insert(id, evidence);
        self.message_key = None;
        self.accounting.bytes = bytes;
        self.accounting.blocks = blocks;
        self.accounting.entries = self.entries.len();
        Ok(())
    }

    fn append_unchecked(
        &mut self,
        role: MessageRole,
        message_id: Option<String>,
        content: ContentBlock,
    ) {
        let key = (role, message_id);
        if self.message_key.as_ref() != Some(&key)
            || !matches!(self.entries.last(), Some(DocumentEntry::Message { role: previous, .. }) if *previous == role)
        {
            self.entries.push(DocumentEntry::Message {
                id: format!("message:{}", self.entries.len()),
                role,
                blocks: vec![],
                delivery: None,
            });
        }
        self.message_key = Some(key);
        if let Some(DocumentEntry::Message {
            blocks, delivery, ..
        }) = self.entries.last_mut()
        {
            if role == MessageRole::User && delivery.is_none() {
                *delivery = delivery_coverage(&content);
            }
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

    fn tool_unchecked(&mut self, id: String, fields: ToolCallUpdateFields) {
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
                    accepted_html_mode: HtmlMode::Static,
                });
                self.entries.len() - 1
            });
        let evidence = self.tools.entry(id).or_default();
        if let Some(title) = fields.title.as_ref() {
            evidence.tool_name = Some(title.clone());
        }
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
            accepted_html_mode,
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
            // Adapters can supply tool results in rawOutput instead of ACP content.
            // Explicit content (including an update clearing it) remains authoritative.
            // Retained raw output is projected on completion even if it arrived earlier.
            if !evidence.explicit_content {
                blocks.clear();
            }
            if !evidence.explicit_content && *status == ToolCallStatus::Completed {
                if let Some(result) = evidence.output.as_ref().and_then(successful_tool_result) {
                    *blocks = result.blocks();
                }
            }
            *accepted_html = if *status == ToolCallStatus::Completed {
                if rich_publication_source(evidence) {
                    accepted_rich_publication(evidence, blocks)
                } else {
                    accepted_publication(evidence, blocks)
                }
            } else {
                None
            };
            *accepted_html_mode = if accepted_html.is_some() && rich_publication_source(evidence) {
                HtmlMode::Interactive
            } else {
                HtmlMode::Static
            };
        }
    }
}

/// Saved App output is presentation evidence only; replay never recreates an
/// MCP source, live Agent session or tool/message/context authority.
fn rich_publication_source(evidence: &ToolEvidence) -> bool {
    evidence.tool_name.as_deref().is_some_and(|name| {
        matches!(
            name,
            "mcp__lens_rich_content__render_html" | "lens_rich_content__render_html"
        )
    }) || evidence.input.as_ref().is_some_and(|input| {
        input.get("server").and_then(Value::as_str) == Some("lens_rich_content")
            || input.get("tool_name").and_then(Value::as_str)
                == Some("lens_rich_content__render_html")
    }) || evidence.output.as_ref().is_some_and(|output| {
        output.get("server_name").and_then(Value::as_str) == Some("lens_rich_content")
    })
}
fn rich_result_body_matches(value: &Value, digest: &str) -> bool {
    value.pointer("/structuredContent/html").is_none_or(|html| {
        html.as_str().is_some_and(|html| {
            let actual: String = Sha256::digest(html.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            actual == digest
        })
    })
}
fn accepted_rich_publication(evidence: &ToolEvidence, blocks: &[DocumentBlock]) -> Option<String> {
    let input = evidence.input.as_ref()?;
    let output = evidence.output.as_ref();
    if let Some(typed) =
        output.filter(|output| output.get("type").and_then(Value::as_str) == Some("MCP"))
    {
        if typed.get("server_name").and_then(Value::as_str) != Some("lens_rich_content")
            || typed.get("tool_name").and_then(Value::as_str) != Some("render_html")
            || typed.get("result").is_some()
        {
            return None;
        }
    }
    let arguments = if input.get("variant").and_then(Value::as_str) == Some("UseTool") {
        if input.get("tool_name").and_then(Value::as_str) != Some("lens_rich_content__render_html")
            || input.get("arguments").is_some()
            || input.get("html").is_some()
        {
            return None;
        }
        input.get("tool_input")?
    } else if input.get("server").is_some() || input.get("tool").is_some() {
        if input.get("server").and_then(Value::as_str) != Some("lens_rich_content")
            || input.get("tool").and_then(Value::as_str) != Some("render_html")
            || input.get("html").is_some()
            || input.get("tool_input").is_some()
        {
            return None;
        }
        input.get("arguments")?
    } else {
        if !matches!(
            evidence.tool_name.as_deref(),
            Some("mcp__lens_rich_content__render_html" | "lens_rich_content__render_html")
        ) {
            return None;
        }
        if input.get("arguments").is_some()
            && (input.get("html").is_some() || input.get("tool_input").is_some())
        {
            return None;
        }
        input.get("arguments").unwrap_or(input)
    };
    let arguments = arguments.as_object().filter(|args| args.len() == 1)?;
    let html = arguments.get("html")?.as_str()?;
    if html.trim().is_empty() || html.len() > MAX_HTML {
        return None;
    }
    let result = match output {
        Some(output) => Some(successful_tool_result(output)?),
        None => None,
    };
    let digest: String = Sha256::digest(html.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if output.is_some_and(|output| {
        !rich_result_body_matches(output.get("result").unwrap_or(output), &digest)
    }) {
        return None;
    }
    let valid = |value: &Value| rich_receipt(value, &digest);
    let accepted = result.as_ref().is_some_and(|result| match result {
        SuccessfulToolResult::Structured(value) => {
            rich_result_body_matches(value, &digest)
                && (valid(value)
                    || value
                        .pointer("/structuredContent/publication")
                        .is_some_and(valid)
                    || value
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|items| {
                            items.iter().any(|item| {
                                item.get("type").and_then(Value::as_str) == Some("text")
                                    && item
                                        .get("text")
                                        .and_then(Value::as_str)
                                        .is_some_and(|text| rich_text_receipt(text, &digest))
                            })
                        }))
        }
        SuccessfulToolResult::Content(items) => items.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("text")
                && item
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| rich_text_receipt(text, &digest))
        }),
        SuccessfulToolResult::Text(text) => rich_text_receipt(text, &digest),
    }) || blocks.iter().any(
        |block| matches!(block,DocumentBlock::Markdown{text} if rich_text_receipt(text,&digest)),
    );
    accepted.then(|| html.to_owned())
}
fn rich_receipt(value: &Value, digest: &str) -> bool {
    value.get("kind").and_then(Value::as_str) == Some("lens_rich_html_publication")
        && value.get("schema_version") == Some(&Value::from(1))
        && receipt(value)
        && value
            .get("turn_id")
            .and_then(Value::as_str)
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok_and(|id| !id.is_nil()))
        && value.get("html_sha256").and_then(Value::as_str) == Some(digest)
}
fn rich_text_receipt(text: &str, digest: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|value| {
        !failed_tool_result(&value)
            && rich_result_body_matches(&value, digest)
            && (rich_receipt(&value, digest)
                || value
                    .pointer("/structuredContent/publication")
                    .is_some_and(|receipt| rich_receipt(receipt, digest))
                || value
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|items| {
                        items.iter().any(|item| {
                            item.get("type").and_then(Value::as_str) == Some("text")
                                && item
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .is_some_and(|text| {
                                        serde_json::from_str::<Value>(text)
                                            .is_ok_and(|receipt| rich_receipt(&receipt, digest))
                                    })
                        })
                    }))
    })
}

/// Borrowed adapter payload shapes; strings are text, never recursive envelopes.
enum SuccessfulToolResult<'a> {
    Structured(&'a Value),
    Content(&'a [Value]),
    Text(&'a str),
}

impl SuccessfulToolResult<'_> {
    fn blocks(&self) -> Vec<DocumentBlock> {
        match self {
            Self::Structured(value) => value
                .get("content")
                .and_then(Value::as_array)
                .map(|items| Self::Content(items).blocks())
                .unwrap_or_default(),
            Self::Content(items) => items
                .iter()
                .map(|item| {
                    serde_json::from_value::<ContentBlock>(item.clone())
                        .map(convert)
                        .unwrap_or_else(|_| DocumentBlock::Unsupported {
                            content_type: "tool result content".into(),
                        })
                })
                .collect(),
            Self::Text(text) => vec![DocumentBlock::Markdown {
                text: (*text).to_owned(),
            }],
        }
    }

    fn has_receipt(&self) -> bool {
        match self {
            Self::Structured(value) => {
                receipt(value)
                    || value
                        .get("content")
                        .and_then(Value::as_array)
                        .is_some_and(|items| Self::Content(items).has_receipt())
            }
            Self::Content(items) => items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some("text")
                    && item
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(text_receipt)
            }),
            Self::Text(text) => text_receipt(text),
        }
    }
}

fn failed_tool_result(output: &Value) -> bool {
    output.get("isError") == Some(&Value::Bool(true))
        || output.get("error").is_some_and(|error| !error.is_null())
}

/// Check errors before normalizing the supported result envelopes and text payloads.
fn successful_tool_result(output: &Value) -> Option<SuccessfulToolResult<'_>> {
    if failed_tool_result(output) {
        return None;
    }
    let result = output.get("result").unwrap_or(output);
    if failed_tool_result(result) {
        return None;
    }
    if result.get("type").and_then(Value::as_str) == Some("MCP") {
        // The supported typed form is direct; combining provider envelopes is ambiguous.
        if output.get("result").is_some() {
            return None;
        }
        // Grok emits a tagged MCP result instead of ACP content. Only its explicit
        // success variant is admitted; error or mixed variants cannot yield content.
        let payload = result.get("output")?.as_object()?;
        if payload.len() != 1 {
            return None;
        }
        let text = payload.get("OkayOutput")?.as_str()?;
        if serde_json::from_str::<Value>(text).is_ok_and(|value| failed_tool_result(&value)) {
            return None;
        }
        return Some(SuccessfulToolResult::Text(text));
    }
    match result {
        Value::Object(_) => Some(SuccessfulToolResult::Structured(result)),
        Value::Array(items) => Some(SuccessfulToolResult::Content(items)),
        Value::String(text) => Some(SuccessfulToolResult::Text(text)),
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

fn text_receipt(text: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|value| receipt(&value))
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
    let output = evidence.output.as_ref();
    if let Some(output) =
        output.filter(|output| output.get("type").and_then(Value::as_str) == Some("MCP"))
    {
        // Typed MCP evidence is bound to this publisher regardless of input shape.
        if output.get("tool_name").and_then(Value::as_str) != Some("publish_html")
            || output.get("server_name").and_then(Value::as_str) != Some("lens_output")
            || output.get("result").is_some()
        {
            return None;
        }
    }
    let input = evidence.input.as_ref()?;
    let input = if input.get("variant").and_then(Value::as_str) == Some("UseTool") {
        // Match the explicit MCP invocation rather than interpreting file paths or
        // searching arbitrary nested tool data for HTML and a receipt.
        if input.get("tool_name").and_then(Value::as_str) != Some("lens_output__publish_html")
            || input.get("arguments").is_some()
            || input.get("html").is_some()
            || output?.get("type").and_then(Value::as_str) != Some("MCP")
        {
            return None;
        }
        input.get("tool_input")?
    } else {
        input.get("arguments").unwrap_or(input)
    };
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
    // Neither a receipt nor a rendered text block can override a failed result.
    let output = match output {
        Some(output) => Some(successful_tool_result(output)?),
        None => None,
    };
    let accepted = output.is_some_and(|output| output.has_receipt())
        || blocks
            .iter()
            .any(|block| matches!(block, DocumentBlock::Markdown { text } if text_receipt(text)));
    accepted.then(|| html.to_owned())
}

/// Historical coverage is presentation evidence from one complete user projection,
/// never execution authority. Unknown, fragmented or future input remains unknown.
fn delivery_coverage(content: &ContentBlock) -> Option<LensDeliveryCoverage> {
    use crate::lens::{MAX_LENS_INPUT_BYTES, MAX_LENS_MEDIA_ATTACHMENTS, MAX_LENS_TARGETS};
    let text = match content {
        ContentBlock::Text(value) => &value.text,
        ContentBlock::Resource(value) => match &value.resource {
            EmbeddedResourceResource::TextResourceContents(value)
                if value.uri.starts_with("lens://projection/")
                    && value.mime_type.as_deref() == Some("application/json") =>
            {
                &value.text
            }
            _ => return None,
        },
        _ => return None,
    };
    // Transport may insert whitespace without changing canonical projection bytes.
    // At most one inserted byte per original byte is admitted before parsing;
    // the semantic JSON still has the unchanged canonical input-size ceiling.
    if text.len() > MAX_LENS_INPUT_BYTES * 2 || !text.trim_start().starts_with('{') {
        return None;
    }
    let value: Value = serde_json::from_str(text).ok()?;
    if serde_json_canonicalizer::to_vec(&value).ok()?.len() > MAX_LENS_INPUT_BYTES {
        return None;
    }
    if value.get("schema_version")?.as_u64()? != 1
        || !value.get("media")?.as_array()?.is_empty()
        || !value.get("media_omissions")?.is_array()
        || !matches!(
            value.get("quality")?.as_str()?,
            "full" | "partial" | "unavailable"
        )
    {
        return None;
    }
    let sources = value.get("sources")?.as_array()?;
    let coverage: LensDeliveryCoverage =
        serde_json::from_value(value.get("delivery")?.clone()).ok()?;
    if sources.is_empty()
        || sources.len() > MAX_LENS_TARGETS
        || coverage.sources.len() != sources.len()
    {
        return None;
    }
    let mut media_ids = std::collections::BTreeSet::new();
    for (index, (source, delivered)) in sources.iter().zip(&coverage.sources).enumerate() {
        let id = format!("source-{index}");
        if source.get("id")?.as_str()? != id || delivered.source_id != id {
            return None;
        }
        match delivered.mode {
            LensDeliveryMode::Complete if !delivered.omitted_media.is_empty() => return None,
            LensDeliveryMode::TextOnlyPartial if delivered.omitted_media.is_empty() => return None,
            _ => {}
        }
        for omitted in &delivered.omitted_media {
            let ordinal = omitted
                .id
                .strip_prefix(&format!("{id}/media-"))?
                .parse::<usize>()
                .ok()?;
            if ordinal >= MAX_LENS_MEDIA_ATTACHMENTS
                || omitted.id != format!("{id}/media-{ordinal}")
                || !media_ids.insert(&omitted.id)
            {
                return None;
            }
        }
    }
    let expected = if coverage
        .sources
        .iter()
        .all(|source| source.mode == LensDeliveryMode::Unavailable)
    {
        LensDeliveryMode::Unavailable
    } else if coverage
        .sources
        .iter()
        .all(|source| source.mode == LensDeliveryMode::Complete)
    {
        LensDeliveryMode::Complete
    } else {
        LensDeliveryMode::TextOnlyPartial
    };
    (coverage.mode == expected && media_ids.len() <= MAX_LENS_MEDIA_ATTACHMENTS).then_some(coverage)
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
    use agent_client_protocol_schema::v1::{ContentChunk, TextContent, ToolCall, ToolCallUpdate};
    fn text(value: &str) -> ContentBlock {
        ContentBlock::Text(TextContent::new(value))
    }
    fn assistant(value: &str) -> SessionUpdate {
        SessionUpdate::AgentMessageChunk(ContentChunk::new(text(value)))
    }
    fn delivery_projection() -> Value {
        serde_json::json!({
            "schema_version":1,"sources":[{"id":"source-0"}],"media":[],
            "media_omissions":[],"quality":"full",
            "delivery":{"mode":"text_only_partial","sources":[{
                "source_id":"source-0","mode":"text_only_partial",
                "omitted_media":[{"id":"source-0/media-0","reason":"image_not_supported"}]
            }]}
        })
    }
    #[test]
    fn delivery_coverage_survives_live_and_replayed_text_or_embedded_user_input() {
        let json = delivery_projection().to_string();
        let resource: ContentBlock = serde_json::from_value(serde_json::json!({
            "type":"resource","resource":{"uri":"lens://projection/1/digest",
            "mimeType":"application/json","text":json}
        }))
        .unwrap();
        for content in [text(&json), resource] {
            let mut live = SessionDocument::default();
            live.append_prompt(&[text("instructions"), content.clone()])
                .unwrap();
            let mut replay = SessionDocument::default();
            for chunk in [text("instructions"), content] {
                replay
                    .record_update(SessionUpdate::UserMessageChunk(ContentChunk::new(chunk)))
                    .unwrap();
            }
            assert_eq!(live, replay);
            assert!(matches!(&replay.entries[0], DocumentEntry::Message {
                delivery: Some(coverage), ..
            } if coverage.mode == LensDeliveryMode::TextOnlyPartial));
            assert_accounting(&live);
            assert_accounting(&replay);
        }
    }
    #[test]
    fn coverage_rejects_malformed_future_or_unrelated_values_and_assistant_claims() {
        for path in ["schema_version", "sources", "delivery"] {
            let mut value = delivery_projection();
            value[path] = serde_json::json!("unknown");
            assert!(delivery_coverage(&text(&value.to_string())).is_none());
        }
        let mut value = delivery_projection();
        value["delivery"]["sources"][0]["source_id"] = serde_json::json!("source-3");
        assert!(delivery_coverage(&text(&value.to_string())).is_none());
        let mut value = delivery_projection();
        value["delivery"]["mode"] = serde_json::json!("complete");
        assert!(delivery_coverage(&text(&value.to_string())).is_none());
        let json = delivery_projection().to_string();
        let mut document = SessionDocument::default();
        document.record_update(assistant(&json)).unwrap();
        assert!(matches!(
            &document.entries[0],
            DocumentEntry::Message { delivery: None, .. }
        ));
        let mut fragmented = SessionDocument::default();
        for part in [&json[..10], &json[10..]] {
            fragmented
                .record_update(SessionUpdate::UserMessageChunk(ContentChunk::new(text(
                    part,
                ))))
                .unwrap();
        }
        assert!(matches!(
            &fragmented.entries[0],
            DocumentEntry::Message { delivery: None, .. }
        ));
    }
    #[test]
    fn coverage_bounds_canonical_bytes_independently_of_transport_whitespace() {
        let mut value = delivery_projection();
        value["padding"] = serde_json::json!("");
        let overhead = serde_json_canonicalizer::to_vec(&value).unwrap().len();
        value["padding"] =
            serde_json::json!("x".repeat(crate::lens::MAX_LENS_INPUT_BYTES - overhead));
        let canonical = serde_json_canonicalizer::to_string(&value).unwrap();
        let transport = serde_json::to_string_pretty(&value).unwrap();
        assert_eq!(canonical.len(), crate::lens::MAX_LENS_INPUT_BYTES);
        assert!(transport.len() > canonical.len());
        assert_eq!(
            delivery_coverage(&text(&canonical)),
            delivery_coverage(&text(&transport))
        );
        assert!(delivery_coverage(&text(&transport)).is_some());
        let oversized_wire = format!(
            "{}{}",
            " ".repeat(crate::lens::MAX_LENS_INPUT_BYTES * 2),
            canonical
        );
        assert!(delivery_coverage(&text(&oversized_wire)).is_none());
        let padding = value["padding"].as_str().unwrap().to_owned() + "x";
        value["padding"] = serde_json::json!(padding);
        assert!(delivery_coverage(&text(&value.to_string())).is_none());
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
                delivery: None,
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
            delivery: None,
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
    fn rich_evidence(html: &str) -> ToolEvidence {
        let digest: String = Sha256::digest(html.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let receipt = serde_json::json!({"kind":"lens_rich_html_publication","schema_version":1,"accepted":true,"publication_id":uuid::Uuid::from_u128(1),"turn_id":uuid::Uuid::from_u128(1),"html_sha256":digest});
        ToolEvidence {
            input: Some(
                serde_json::json!({"server":"lens_rich_content","tool":"render_html","arguments":{"html":html}}),
            ),
            output: Some(
                serde_json::json!({"result":{"content":[{"type":"text","text":receipt.to_string()}],"structuredContent":{"html":html,"publication":receipt}},"error":null}),
            ),
            ..Default::default()
        }
    }
    #[test]
    fn rich_html_receipt_checks_source_success_hash_and_capacity() {
        let original = rich_evidence("<p>Saved</p><script>never replay authority</script>");
        let html = original.input.as_ref().unwrap()["arguments"]["html"]
            .as_str()
            .unwrap();
        assert_eq!(
            accepted_rich_publication(&original, &[]).as_deref(),
            Some(html)
        );
        for change in [
            "source",
            "tool",
            "hash",
            "body",
            "failed",
            "missing",
            "arguments",
        ] {
            let mut evidence = original.clone();
            match change {
                "source" => evidence.input.as_mut().unwrap()["server"] = serde_json::json!("other"),
                "tool" => evidence.input.as_mut().unwrap()["tool"] = serde_json::json!("other"),
                "hash" => {
                    evidence.input.as_mut().unwrap()["arguments"]["html"] =
                        serde_json::json!("<p>Changed</p>")
                }
                "body" => {
                    evidence.output.as_mut().unwrap()["result"]["structuredContent"]["html"] =
                        serde_json::json!("<p>Conflicting</p>")
                }
                "failed" => {
                    evidence.output.as_mut().unwrap()["result"]["isError"] = serde_json::json!(true)
                }
                "missing" => evidence.output = Some(serde_json::json!({"result":{}})),
                "arguments" => {
                    evidence.input.as_mut().unwrap()["arguments"]["path"] =
                        serde_json::json!("/tmp/generated.html")
                }
                _ => unreachable!(),
            }
            assert!(
                accepted_rich_publication(&evidence, &[]).is_none(),
                "{change}"
            );
        }
        assert!(
            accepted_rich_publication(&rich_evidence(&"x".repeat(MAX_HTML + 1)), &[]).is_none()
        );
        assert!(accepted_rich_publication(&rich_evidence(" "), &[]).is_none());
    }
    #[test]
    fn rich_receipt_supported_provider_shapes_and_invalid_receipt_cannot_use_legacy_reader() {
        let original = rich_evidence("<p>Provider saved body</p>");
        let args = original.input.as_ref().unwrap()["arguments"].clone();
        let result = original.output.as_ref().unwrap()["result"].clone();
        let receipt = result["structuredContent"]["publication"].clone();
        for evidence in [
            ToolEvidence {
                input: Some(args.clone()),
                tool_name: Some("mcp__lens_rich_content__render_html".into()),
                output: Some(result.clone()),
                ..Default::default()
            },
            ToolEvidence {
                input: Some(
                    serde_json::json!({"variant":"UseTool","tool_name":"lens_rich_content__render_html","tool_input":args}),
                ),
                output: Some(
                    serde_json::json!({"type":"MCP","server_name":"lens_rich_content","tool_name":"render_html","output":{"OkayOutput":result.to_string()}}),
                ),
                ..Default::default()
            },
        ] {
            assert_eq!(
                accepted_rich_publication(&evidence, &[]).as_deref(),
                Some("<p>Provider saved body</p>")
            );
        }
        for field in [
            "kind",
            "schema_version",
            "publication_id",
            "turn_id",
            "html_sha256",
        ] {
            let mut invalid = receipt.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(
                !rich_text_receipt(
                    &invalid.to_string(),
                    receipt["html_sha256"].as_str().unwrap()
                ),
                "{field}"
            );
        }
        let mut doc = SessionDocument::default();
        doc.tool_unchecked("invalid".into(),ToolCallUpdateFields::new().title("mcp__lens_rich_content__render_html").raw_input(serde_json::json!({"html":"<p>Legacy-shaped input</p>","turn_id":uuid::Uuid::from_u128(1)})).raw_output(serde_json::json!({"accepted":true,"publication_id":uuid::Uuid::from_u128(1)})).status(ToolCallStatus::Completed));
        assert!(matches!(
            &doc.entries[0],
            DocumentEntry::Tool {
                accepted_html: None,
                ..
            }
        ));
    }
    #[test]
    fn rich_html_receipts_are_correlated_per_tool_call_and_never_grant_live_authority() {
        let mut document = SessionDocument::default();
        let first = rich_evidence("<p>First</p>");
        let second = rich_evidence("<p>Second</p>");
        document.tool_unchecked(
            "first".into(),
            ToolCallUpdateFields::new()
                .title("mcp__lens_rich_content__render_html")
                .raw_input(first.input.unwrap())
                .raw_output(second.output.clone().unwrap())
                .status(ToolCallStatus::Completed),
        );
        document.tool_unchecked(
            "second".into(),
            ToolCallUpdateFields::new()
                .title("mcp__lens_rich_content__render_html")
                .raw_input(second.input.unwrap())
                .raw_output(second.output.unwrap())
                .status(ToolCallStatus::Completed),
        );
        assert!(matches!(
            &document.entries[0],
            DocumentEntry::Tool {
                accepted_html: None,
                ..
            }
        ));
        assert!(
            matches!(&document.entries[1], DocumentEntry::Tool{accepted_html:Some(html),..} if html=="<p>Second</p>")
        );
        let serialized = serde_json::to_value(&document).unwrap();
        assert!(serialized.to_string().contains("accepted_html"));
        assert_eq!(
            serialized["entries"][1]["accepted_html_mode"],
            "interactive"
        );
        let restored: SessionDocument = serde_json::from_value(serialized.clone()).unwrap();
        assert!(matches!(
            &restored.entries[1],
            DocumentEntry::Tool {
                accepted_html_mode: HtmlMode::Interactive,
                ..
            }
        ));
        let mut old_history = serialized.clone();
        old_history["entries"][1]
            .as_object_mut()
            .unwrap()
            .remove("accepted_html_mode");
        let old_restored: SessionDocument = serde_json::from_value(old_history).unwrap();
        assert!(matches!(
            &old_restored.entries[1],
            DocumentEntry::Tool {
                accepted_html: Some(_),
                accepted_html_mode: HtmlMode::Static,
                ..
            }
        ));
        assert!(matches!(
            &document.entries[0],
            DocumentEntry::Tool {
                accepted_html_mode: HtmlMode::Static,
                ..
            }
        ));
        assert!(!serialized.to_string().contains("host_capabilities"));
        assert!(!serialized.to_string().contains("session_id"));
    }
    fn codex_result(content: Value) -> Value {
        serde_json::json!({"result":{"content":content},"error":null})
    }

    #[test]
    fn claude_receipt_array_and_string_restore_html_on_sparse_completion() {
        let receipt = serde_json::json!({"accepted":true,"publication_id":"e9e57747-88ed-47b7-a301-47f6c13e4503"}).to_string();
        for output in [
            serde_json::json!([{"type":"text","text":receipt}]),
            serde_json::json!(receipt),
        ] {
            let mut document = SessionDocument::default();
            document.record_update(SessionUpdate::ToolCall(
                ToolCall::new("publication", "mcp__lens_output__publish_html")
                    .status(ToolCallStatus::InProgress)
                    .raw_input(serde_json::json!({"html":"<h1>Claude</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}))
                    .raw_output(output),
            )).unwrap();
            document
                .record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    "publication",
                    ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
                )))
                .unwrap();
            assert!(
                matches!(&document.entries[0], DocumentEntry::Tool { blocks, accepted_html: Some(html), .. }
                if html == "<h1>Claude</h1>" && blocks == &vec![DocumentBlock::Markdown { text: receipt.clone() }])
            );
            assert_accounting(&document);
        }
    }

    #[test]
    fn failed_or_unsupported_results_cannot_be_overridden_by_receipt_content() {
        let accepted = serde_json::json!({"accepted":true,"publication_id":"e9e57747-88ed-47b7-a301-47f6c13e4503"});
        let blocks = [DocumentBlock::Markdown {
            text: accepted.to_string(),
        }];
        for output in [
            Value::Null,
            serde_json::json!(true),
            serde_json::json!(42),
            serde_json::json!({"result":null,"error":null}),
            serde_json::json!({"isError":true,"content":[{"type":"text","text":accepted.to_string()}]}),
            serde_json::json!({"result":accepted,"error":"failed"}),
            serde_json::json!({"result":accepted,"isError":true}),
            serde_json::json!({"result":{"isError":true,"accepted":true,"publication_id":accepted["publication_id"]}}),
            serde_json::json!({"result":{"error":"failed","accepted":true,"publication_id":accepted["publication_id"]}}),
        ] {
            let evidence = ToolEvidence {
                input: Some(
                    serde_json::json!({"html":"<h1>Rejected</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}),
                ),
                output: Some(output),
                ..Default::default()
            };
            assert!(accepted_publication(&evidence, &blocks).is_none());
        }
    }

    #[test]
    fn claude_receipts_require_completed_status_and_valid_receipt() {
        let receipt = serde_json::json!({"accepted":true,"publication_id":"e9e57747-88ed-47b7-a301-47f6c13e4503"}).to_string();
        for output in [
            serde_json::json!([{"type":"text","text":receipt}]),
            serde_json::json!(receipt),
        ] {
            let mut document = SessionDocument::default();
            document.record_update(SessionUpdate::ToolCall(
                ToolCall::new("publication", "publish_html")
                    .status(ToolCallStatus::Completed)
                    .raw_input(serde_json::json!({"html":"<h1>Accepted</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}))
                    .raw_output(output),
            )).unwrap();
            assert!(matches!(
                &document.entries[0],
                DocumentEntry::Tool {
                    accepted_html: Some(_),
                    ..
                }
            ));
            document
                .record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    "publication",
                    ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
                )))
                .unwrap();
            assert!(
                matches!(&document.entries[0], DocumentEntry::Tool { blocks, accepted_html: None, .. } if blocks.is_empty())
            );
            for invalid in [
                "not JSON".to_string(),
                serde_json::json!({"accepted":false,"publication_id":"e9e57747-88ed-47b7-a301-47f6c13e4503"}).to_string(),
                serde_json::json!({"accepted":true,"publication_id":"00000000-0000-0000-0000-000000000000"}).to_string(),
            ] {
                document.record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                    "publication", ToolCallUpdateFields::new().status(ToolCallStatus::Completed)
                        .raw_output(serde_json::json!([{"type":"text","text":invalid}])),
                ))).unwrap();
                assert!(matches!(&document.entries[0], DocumentEntry::Tool { accepted_html: None, .. }));
            }
        }
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
    fn grok_tagged_mcp_replay_restores_html_after_sparse_completion() {
        // Anonymized shape of Grok Build 1.0.44's persisted session/update events:
        // file indirection, resolved UseTool input, then completed MCP OkayOutput.
        let updates = [
            serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"publication",
                "title":"use_tool","rawInput":{"file":"/tmp/publication.json"}}),
            serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"publication",
                "kind":"read","title":"Read MCP source","rawInput":{"source_file":"/tmp/publication.json"}}),
            serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"publication",
                "kind":"other","title":"lens_output__publish_html",
                "rawInput":{"variant":"UseTool","tool_name":"lens_output__publish_html",
                    "tool_input":{"html":"<h1>Recovered</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}}}),
            serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"publication",
                "status":"completed","rawOutput":{"type":"MCP","tool_name":"publish_html",
                    "server_name":"lens_output","output":{"OkayOutput":"{\"accepted\":true,\"publication_id\":\"e9e57747-88ed-47b7-a301-47f6c13e4503\"}"}}}),
        ];
        let mut live = SessionDocument::default();
        for update in &updates[..3] {
            live.record_update(serde_json::from_value(update.clone()).unwrap())
                .unwrap();
            assert!(matches!(
                &live.entries[0],
                DocumentEntry::Tool {
                    accepted_html: None,
                    ..
                }
            ));
        }
        live.record_update(serde_json::from_value(updates[3].clone()).unwrap())
            .unwrap();
        let mut replay = SessionDocument::default();
        for update in updates {
            replay
                .record_update(serde_json::from_value(update).unwrap())
                .unwrap();
        }
        assert!(
            matches!(&replay.entries[0], DocumentEntry::Tool { accepted_html: Some(html), blocks, .. }
            if html == "<h1>Recovered</h1>" && matches!(&blocks[..], [DocumentBlock::Markdown { text }] if text_receipt(text)))
        );
        assert_eq!(
            serde_json::to_value(&live).unwrap(),
            serde_json::to_value(&replay).unwrap()
        );
        assert_accounting(&replay);
    }

    #[test]
    fn grok_tagged_publication_rejects_errors_ambiguity_and_invalid_identity() {
        let input = serde_json::json!({"variant":"UseTool","tool_name":"lens_output__publish_html",
            "tool_input":{"html":"<h1>Recovered</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}});
        let output = serde_json::json!({"type":"MCP","tool_name":"publish_html","server_name":"lens_output",
            "output":{"OkayOutput":"{\"accepted\":true,\"publication_id\":\"e9e57747-88ed-47b7-a301-47f6c13e4503\"}"}});
        let accepted_block = DocumentBlock::Markdown {
            text: "{\"accepted\":true,\"publication_id\":\"e9e57747-88ed-47b7-a301-47f6c13e4503\"}"
                .into(),
        };
        let reject = |input: Value, output: Value| {
            let evidence = ToolEvidence {
                input: Some(input),
                output: Some(output),
                ..Default::default()
            };
            assert!(
                accepted_publication(&evidence, std::slice::from_ref(&accepted_block)).is_none()
            );
        };
        for (field, value) in [
            ("type", serde_json::json!("Bash")),
            ("server_name", serde_json::json!("other_server")),
            ("tool_name", serde_json::json!("other_tool")),
            ("error", serde_json::json!("failed")),
            ("isError", serde_json::json!(true)),
            ("result", serde_json::json!({"accepted":true})),
            ("output", serde_json::json!({"ErrorOutput":"failed"})),
            (
                "output",
                serde_json::json!({"OkayOutput":"{}","ErrorOutput":"failed"}),
            ),
            (
                "output",
                serde_json::json!({"OkayOutput":{"accepted":true}}),
            ),
        ] {
            let mut bad = output.clone();
            bad[field] = value;
            reject(input.clone(), bad);
        }
        for (field, value) in [
            ("tool_name", serde_json::json!("other_tool")),
            ("arguments", serde_json::json!({"html":"other"})),
            ("html", serde_json::json!("other")),
            (
                "tool_input",
                serde_json::json!({"html":"<h1>Recovered</h1>","turn_id":"00000000-0000-0000-0000-000000000000"}),
            ),
            (
                "tool_input",
                serde_json::json!({"html":"<h1>Recovered</h1>"}),
            ),
            (
                "tool_input",
                serde_json::json!({"html":" ","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}),
            ),
        ] {
            let mut bad = input.clone();
            bad[field] = value;
            reject(bad, output.clone());
        }
        for receipt in [
            "invalid JSON",
            "{\"accepted\":false}",
            "{\"accepted\":true,\"publication_id\":\"00000000-0000-0000-0000-000000000000\"}",
        ] {
            let mut bad = output.clone();
            bad["output"]["OkayOutput"] = serde_json::json!(receipt);
            let evidence = ToolEvidence {
                input: Some(input.clone()),
                output: Some(bad),
                ..Default::default()
            };
            assert!(accepted_publication(&evidence, &[]).is_none());
        }
    }

    #[test]
    fn grok_tagged_result_inner_failure_cannot_be_overridden_by_content() {
        let input = serde_json::json!({"variant":"UseTool","tool_name":"lens_output__publish_html",
            "tool_input":{"html":"<h1>Rejected</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}});
        let block = DocumentBlock::Markdown {
            text: "{\"accepted\":true,\"publication_id\":\"e9e57747-88ed-47b7-a301-47f6c13e4503\"}"
                .into(),
        };
        for failure in [
            serde_json::json!({"isError":true}),
            serde_json::json!({"error":"failed"}),
        ] {
            let mut receipt = serde_json::json!({"accepted":true,"publication_id":"e9e57747-88ed-47b7-a301-47f6c13e4503"});
            receipt
                .as_object_mut()
                .unwrap()
                .extend(failure.as_object().unwrap().clone());
            let evidence = ToolEvidence {
                input: Some(input.clone()),
                output: Some(serde_json::json!({
                "type":"MCP","tool_name":"publish_html","server_name":"lens_output",
                "output":{"OkayOutput":receipt.to_string()}})),
                ..Default::default()
            };
            assert!(accepted_publication(&evidence, std::slice::from_ref(&block)).is_none());
        }
    }

    #[test]
    fn grok_tagged_publication_retains_evidence_on_null_sparse_update() {
        let mut document = SessionDocument::default();
        for update in [
            serde_json::json!({"sessionUpdate":"tool_call","toolCallId":"publication","title":"lens_output__publish_html",
                "status":"in_progress","rawInput":{"variant":"UseTool","tool_name":"lens_output__publish_html",
                    "tool_input":{"html":"<h1>Retained</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"}},
                "rawOutput":{"type":"MCP","tool_name":"publish_html","server_name":"lens_output",
                    "output":{"OkayOutput":"{\"accepted\":true,\"publication_id\":\"e9e57747-88ed-47b7-a301-47f6c13e4503\"}"}}}),
            serde_json::json!({"sessionUpdate":"tool_call_update","toolCallId":"publication","status":"completed",
                "rawInput":null,"rawOutput":null}),
        ] {
            document
                .record_update(serde_json::from_value(update).unwrap())
                .unwrap();
        }
        assert!(
            matches!(&document.entries[0], DocumentEntry::Tool { accepted_html: Some(html), .. } if html == "<h1>Retained</h1>")
        );
        assert_accounting(&document);
    }

    #[test]
    fn typed_mcp_publication_identity_is_checked_for_legacy_input_shapes() {
        let arguments = serde_json::json!({"html":"<h1>Rejected</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"});
        let receipt =
            "{\"accepted\":true,\"publication_id\":\"e9e57747-88ed-47b7-a301-47f6c13e4503\"}";
        for input in [
            arguments.clone(),
            serde_json::json!({"arguments":arguments}),
        ] {
            for (server, tool) in [
                ("other_server", "publish_html"),
                ("lens_output", "other_tool"),
            ] {
                let evidence = ToolEvidence {
                    input: Some(input.clone()),
                    output: Some(serde_json::json!({
                    "type":"MCP","tool_name":tool,"server_name":server,"output":{"OkayOutput":receipt}})),
                    ..Default::default()
                };
                assert!(accepted_publication(
                    &evidence,
                    &[DocumentBlock::Markdown {
                        text: receipt.into()
                    }]
                )
                .is_none());
            }
        }
    }

    #[test]
    fn nested_typed_mcp_result_is_not_a_supported_publication_envelope() {
        let input = serde_json::json!({"html":"<h1>Rejected</h1>","turn_id":"4cb69bd1-082a-4a73-977d-2da51db59a5a"});
        let receipt =
            "{\"accepted\":true,\"publication_id\":\"e9e57747-88ed-47b7-a301-47f6c13e4503\"}";
        for server in ["lens_output", "other_server"] {
            let evidence = ToolEvidence {
                input: Some(input.clone()),
                output: Some(serde_json::json!({"result":{
                "type":"MCP","tool_name":"publish_html","server_name":server,"output":{"OkayOutput":receipt}}})),
                ..Default::default()
            };
            assert!(accepted_publication(
                &evidence,
                &[DocumentBlock::Markdown {
                    text: receipt.into()
                }]
            )
            .is_none());
        }
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
        for output in [
            codex_result(serde_json::json!([{"type":"text","text":"raw"}])),
            serde_json::json!([{"type":"text","text":"raw"}]),
            serde_json::json!("raw"),
        ] {
            let mut document = SessionDocument::default();
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
    fn assert_accounting(document: &SessionDocument) {
        let actual = wire_size(document)
            + wire_size(
                &document
                    .tools
                    .iter()
                    .map(|(id, e)| (id, &e.input, &e.output))
                    .collect::<Vec<_>>(),
            );
        assert_eq!(document.accounting.bytes, actual);
        assert_eq!(
            document.accounting.blocks,
            document.entries.iter().map(block_count).sum::<usize>()
        );
    }

    #[test]
    fn incremental_accounting_matches_wire_after_mixed_updates_and_deserialization() {
        let mut doc = SessionDocument::default();
        for update in [
            assistant("quote\" newline\n \u{65e5}\u{672c}\u{8a9e}"),
            assistant("\t\u{0} repeated"),
            SessionUpdate::ToolCall(
                ToolCall::new("tool", "title\"")
                    .raw_input(serde_json::json!({"nested":["\n",null,4]})),
            ),
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "tool",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .content(vec![text("answer").into()]),
            )),
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "tool",
                ToolCallUpdateFields::new().content(vec![]),
            )),
            assistant("after tool"),
        ] {
            doc.record_update(update).unwrap();
            assert_accounting(&doc);
        }
        doc.append_prompt(&[text("one"), text("two")]).unwrap();
        assert_accounting(&doc);
        let mut restored: SessionDocument =
            serde_json::from_value(serde_json::to_value(&doc).unwrap()).unwrap();
        restored.record_update(assistant("restored")).unwrap();
        assert_accounting(&restored);
        restored
            .record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "tool",
                ToolCallUpdateFields::new().title("restored title"),
            )))
            .unwrap();
        assert_accounting(&restored);
    }

    #[test]
    fn content_change_summary_excludes_ignored_empty_and_evidence_only_updates() {
        let mut doc = SessionDocument::default();
        doc.record_update(assistant("first")).unwrap();
        assert_eq!(doc.last_changed_entry(), Some(0));
        doc.record_update(assistant("")).unwrap();
        assert_eq!(doc.last_changed_entry(), None);
        doc.record_update(SessionUpdate::AgentThoughtChunk(ContentChunk::new(text(
            "private",
        ))))
        .unwrap();
        assert_eq!(doc.last_changed_entry(), None);
        doc.record_update(SessionUpdate::ToolCall(ToolCall::new("tool", "Tool")))
            .unwrap();
        assert_eq!(doc.last_changed_entry(), Some(1));
        doc.record_update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "tool",
            ToolCallUpdateFields::new().raw_input(serde_json::json!({"value":1})),
        )))
        .unwrap();
        assert_eq!(doc.last_changed_entry(), None);
        assert_accounting(&doc);
    }

    #[test]
    fn exact_byte_limit_accepts_boundary_and_rejects_append_atomically() {
        let mut doc = SessionDocument::default();
        doc.record_update(assistant("")).unwrap();
        let available = MAX_BYTES - doc.accounting.bytes;
        doc.record_update(assistant(&"x".repeat(available)))
            .unwrap();
        assert_eq!(doc.accounting.bytes, MAX_BYTES);
        let before_key = doc.message_key.clone();
        assert!(doc.record_update(assistant("\n")).is_err());
        assert_eq!(doc.accounting.bytes, MAX_BYTES);
        assert_eq!(doc.message_key, before_key);
        assert_eq!(doc.last_changed_entry(), None);
        assert!(doc
            .record_update(SessionUpdate::ToolCall(
                ToolCall::new("rejected", "Tool").raw_input(serde_json::json!({"input":"value"}))
            ))
            .is_err());
        assert!(doc.tools.is_empty());
        assert!(doc.accounting.tool_indices.is_empty());
        assert_eq!(doc.message_key, before_key);
        assert_eq!(doc.entries.len(), 1);
        assert_accounting(&doc);
    }

    #[test]
    fn append_work_scales_with_incoming_text() {
        for count in [100, 1000, 3000] {
            let mut doc = SessionDocument::default();
            for _ in 0..count {
                doc.record_update(assistant(&"x".repeat(256))).unwrap();
            }
            assert_accounting(&doc);
            assert_eq!(doc.entries.len(), 1);
        }
    }
}
