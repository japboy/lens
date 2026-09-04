use crate::model::LensOutputBlock;
use agent_client_protocol::{
    schema::v1::{
        ContentBlock, ContentChunk, SessionUpdate, ToolCallContent, ToolCallId, ToolCallStatus,
    },
    Error,
};
use base64::prelude::*;

const MAX_INLINE_IMAGE_DECODED_BYTES: usize = 10 * 1024 * 1024;
const MAX_INLINE_IMAGE_ENCODED_BYTES: usize = MAX_INLINE_IMAGE_DECODED_BYTES.div_ceil(3) * 4;
const MAX_TOOL_CALLS: usize = 256;
const MAX_TOOL_IMAGE_BLOCKS: usize = 32;
const MAX_TOOL_IMAGE_ENCODED_BYTES: usize = 64 * 1024 * 1024;

/// One prompt turn owns the ordered message segments and tool-result snapshots.
/// Tool updates replace their content at the first notification's position.
#[derive(Debug, Default)]
pub(crate) struct AgentOutputCandidate {
    entries: Vec<OutputEntry>,
    pub(crate) received_updates: usize,
}

#[derive(Debug)]
enum OutputEntry {
    Message(Vec<LensOutputBlock>),
    Tool(ToolOutput),
}

#[derive(Debug)]
struct ToolOutput {
    id: ToolCallId,
    status: ToolCallStatus,
    images: Vec<LensOutputBlock>,
}

impl ToolOutput {
    fn visible_images(&self) -> &[LensOutputBlock] {
        if self.status == ToolCallStatus::Completed {
            &self.images
        } else {
            &[]
        }
    }
}

impl AgentOutputCandidate {
    /// Returns whether the displayable content changed. Protocol-only updates
    /// still advance the received-update count without becoming Interpretation.
    pub(crate) fn record_update(
        &mut self,
        update: SessionUpdate,
        safe_mode_id: &str,
    ) -> Result<bool, Error> {
        self.received_updates = self.received_updates.checked_add(1).ok_or_else(|| {
            Error::internal_error().data("Agent update count exceeded the finite local limit")
        })?;
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                let block = lens_output_block(chunk);
                if matches!(&block, LensOutputBlock::Markdown { text, .. } if text.is_empty()) {
                    return Ok(false);
                }
                if let Some(OutputEntry::Message(blocks)) = self.entries.last_mut() {
                    push_output_block(blocks, block);
                } else {
                    self.entries.push(OutputEntry::Message(vec![block]));
                }
                Ok(true)
            }
            SessionUpdate::ToolCall(call) => {
                self.update_tool(call.tool_call_id, Some(call.status), Some(call.content))
            }
            SessionUpdate::ToolCallUpdate(update) => self.update_tool(
                update.tool_call_id,
                update.fields.status,
                update.fields.content,
            ),
            SessionUpdate::CurrentModeUpdate(update)
                if update.current_mode_id.to_string() != safe_mode_id =>
            {
                Err(Error::invalid_params().data(format!(
                    "ACP session left required safe mode {safe_mode_id}; refusing to continue"
                )))
            }
            _ => Ok(false),
        }
    }

    fn update_tool(
        &mut self,
        id: ToolCallId,
        status: Option<ToolCallStatus>,
        content: Option<Vec<ToolCallContent>>,
    ) -> Result<bool, Error> {
        // Keep only typed images, never revised prompts, rawOutput, resources,
        // tool arguments, diffs, or terminal output. Image URIs are provenance.
        let images = content.map(|content| {
            content
                .into_iter()
                .filter_map(|content| match content {
                    ToolCallContent::Content(content)
                        if matches!(content.content, ContentBlock::Image(_)) =>
                    {
                        Some(lens_output_block(ContentChunk::new(content.content)))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        });
        let existing = self
            .entries
            .iter()
            .position(|entry| matches!(entry, OutputEntry::Tool(tool) if tool.id == id));
        let mut tool_count = 0;
        let mut image_count = 0;
        let mut image_bytes = 0;
        for (index, entry) in self.entries.iter().enumerate() {
            if let OutputEntry::Tool(tool) = entry {
                tool_count += 1;
                if Some(index) != existing || images.is_none() {
                    image_count += tool.images.len();
                    image_bytes += encoded_image_bytes(&tool.images);
                }
            }
        }
        if let Some(images) = &images {
            image_count += images.len();
            image_bytes += encoded_image_bytes(images);
        }
        if (existing.is_none() && tool_count >= MAX_TOOL_CALLS)
            || image_count > MAX_TOOL_IMAGE_BLOCKS
            || image_bytes > MAX_TOOL_IMAGE_ENCODED_BYTES
        {
            return Err(
                Error::invalid_params().data("Agent tool output exceeded the per-turn limit")
            );
        }

        if let Some(index) = existing {
            let OutputEntry::Tool(tool) = &mut self.entries[index] else {
                unreachable!("tool position was selected above")
            };
            let next_status = status.unwrap_or(tool.status);
            let next_visible = if next_status == ToolCallStatus::Completed {
                images.as_deref().unwrap_or(&tool.images)
            } else {
                &[]
            };
            let changed = tool.visible_images() != next_visible;
            tool.status = next_status;
            if let Some(images) = images {
                // Omitted content preserves the snapshot; an explicit empty
                // collection clears it, as required by ACP ToolCallUpdate.
                tool.images = images;
            }
            Ok(changed)
        } else {
            let tool = ToolOutput {
                id,
                status: status.unwrap_or_default(),
                images: images.unwrap_or_default(),
            };
            let changed = !tool.visible_images().is_empty();
            self.entries.push(OutputEntry::Tool(tool));
            Ok(changed)
        }
    }

    pub(crate) fn blocks(&self) -> Vec<LensOutputBlock> {
        let mut blocks = Vec::new();
        for entry in &self.entries {
            let content = match entry {
                OutputEntry::Message(content) => content,
                OutputEntry::Tool(tool) => tool.visible_images(),
            };
            for block in content {
                push_output_block(&mut blocks, block.clone());
            }
        }
        blocks
    }

    pub(crate) fn has_output(&self) -> bool {
        self.entries.iter().any(|entry| match entry {
            OutputEntry::Message(blocks) => blocks.iter().any(|block| match block {
                LensOutputBlock::Markdown { text, .. } => !text.trim().is_empty(),
                LensOutputBlock::Image { .. } | LensOutputBlock::Unsupported { .. } => true,
            }),
            OutputEntry::Tool(tool) => !tool.visible_images().is_empty(),
        })
    }

    #[cfg(test)]
    pub(crate) fn from_blocks(blocks: Vec<LensOutputBlock>, received_updates: usize) -> Self {
        Self {
            entries: vec![OutputEntry::Message(blocks)],
            received_updates,
        }
    }
}

fn encoded_image_bytes(blocks: &[LensOutputBlock]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            LensOutputBlock::Image { data, .. } => data.len(),
            _ => 0,
        })
        .sum()
}

fn push_output_block(blocks: &mut Vec<LensOutputBlock>, block: LensOutputBlock) {
    if let LensOutputBlock::Markdown { message_id, text } = &block {
        if text.is_empty() {
            return;
        }
        if let Some(LensOutputBlock::Markdown {
            message_id: previous_id,
            text: previous_text,
        }) = blocks.last_mut()
        {
            if previous_id == message_id {
                previous_text.push_str(text);
                return;
            }
        }
    }
    blocks.push(block);
}

fn lens_output_block(chunk: ContentChunk) -> LensOutputBlock {
    let message_id = chunk.message_id.map(|message_id| message_id.to_string());
    match chunk.content {
        ContentBlock::Text(content) => LensOutputBlock::Markdown {
            message_id,
            text: content.text,
        },
        ContentBlock::Image(content)
            if is_supported_image_mime_type(&content.mime_type)
                && is_valid_inline_image_data(&content.data) =>
        {
            LensOutputBlock::Image {
                message_id,
                mime_type: content.mime_type.to_ascii_lowercase(),
                data: content.data,
                uri: content.uri,
            }
        }
        ContentBlock::Image(content) => LensOutputBlock::Unsupported {
            message_id,
            content_type: format!("image ({})", content.mime_type),
        },
        ContentBlock::Audio(content) => LensOutputBlock::Unsupported {
            message_id,
            content_type: format!("audio ({})", content.mime_type),
        },
        ContentBlock::ResourceLink(_) => LensOutputBlock::Unsupported {
            message_id,
            content_type: "resource_link".into(),
        },
        ContentBlock::Resource(_) => LensOutputBlock::Unsupported {
            message_id,
            content_type: "resource".into(),
        },
        _ => LensOutputBlock::Unsupported {
            message_id,
            content_type: "unknown".into(),
        },
    }
}

pub(crate) fn is_supported_image_mime_type(mime_type: &str) -> bool {
    matches!(
        mime_type.to_ascii_lowercase().as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    )
}

pub(crate) fn is_valid_inline_image_data(data: &str) -> bool {
    if data.is_empty() || data.len() > MAX_INLINE_IMAGE_ENCODED_BYTES {
        return false;
    }
    BASE64_STANDARD
        .decode(data)
        .is_ok_and(|decoded| decoded.len() <= MAX_INLINE_IMAGE_DECODED_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        ImageContent, TextContent, ToolCall, ToolCallUpdate, ToolCallUpdateFields,
    };

    const SAFE_MODE: &str = "read-only";

    fn image(data: &str) -> ToolCallContent {
        ContentBlock::Image(
            ImageContent::new(data, "image/png").uri("file:///unread/generated.png"),
        )
        .into()
    }

    fn text(text: &str, message: &str) -> SessionUpdate {
        SessionUpdate::AgentMessageChunk(
            ContentChunk::new(ContentBlock::Text(TextContent::new(text))).message_id(message),
        )
    }

    fn update(id: &str, fields: ToolCallUpdateFields) -> SessionUpdate {
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(id.to_owned(), fields))
    }

    fn completed(id: &str, content: Vec<ToolCallContent>) -> SessionUpdate {
        SessionUpdate::ToolCall(
            ToolCall::new(id.to_owned(), "Image generation")
                .status(ToolCallStatus::Completed)
                .content(content),
        )
    }

    fn image_data(candidate: &AgentOutputCandidate) -> Vec<String> {
        candidate
            .blocks()
            .into_iter()
            .filter_map(|block| match block {
                LensOutputBlock::Image { data, .. } => Some(data),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn completed_tool_snapshot_renders_images_without_exposing_tool_prose() {
        let mut candidate = AgentOutputCandidate::default();
        candidate
            .record_update(
                completed(
                    "image-1",
                    vec![
                        ContentBlock::Text(TextContent::new("Revised prompt: private tool detail"))
                            .into(),
                        image("aW1hZ2U="),
                        image("c2Vjb25k"),
                    ],
                ),
                SAFE_MODE,
            )
            .unwrap();
        assert!(candidate.has_output());
        assert_eq!(image_data(&candidate), ["aW1hZ2U=", "c2Vjb25k"]);
        assert_eq!(candidate.blocks().len(), 2);
        assert!(matches!(&candidate.blocks()[0], LensOutputBlock::Image {
            message_id: None, uri: Some(uri), ..
        } if uri == "file:///unread/generated.png"));
    }

    #[test]
    fn status_only_completion_publishes_retained_images_at_the_tool_position() {
        let mut candidate = AgentOutputCandidate::default();
        candidate
            .record_update(text("Before", "before"), SAFE_MODE)
            .unwrap();
        candidate
            .record_update(
                SessionUpdate::ToolCall(
                    ToolCall::new("image-1", "Generating")
                        .status(ToolCallStatus::InProgress)
                        .content(vec![image("aW1hZ2U=")]),
                ),
                SAFE_MODE,
            )
            .unwrap();
        candidate
            .record_update(text("After", "after"), SAFE_MODE)
            .unwrap();
        assert!(image_data(&candidate).is_empty());
        assert!(candidate
            .record_update(
                update(
                    "image-1",
                    ToolCallUpdateFields::new().status(ToolCallStatus::Completed)
                ),
                SAFE_MODE
            )
            .unwrap());
        assert!(matches!(candidate.blocks().as_slice(), [
            LensOutputBlock::Markdown { text: before, .. },
            LensOutputBlock::Image { .. },
            LensOutputBlock::Markdown { text: after, .. }
        ] if before == "Before" && after == "After"));
    }

    #[test]
    fn completed_update_without_start_and_repeated_updates_do_not_duplicate_images() {
        let mut candidate = AgentOutputCandidate::default();
        let event = update(
            "image-1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Completed)
                .content(vec![image("aW1hZ2U=")]),
        );
        assert!(candidate.record_update(event.clone(), SAFE_MODE).unwrap());
        candidate
            .record_update(text("After", "after"), SAFE_MODE)
            .unwrap();
        assert!(!candidate.record_update(event, SAFE_MODE).unwrap());
        assert!(!candidate
            .record_update(
                update(
                    "image-1",
                    ToolCallUpdateFields::new().title("Metadata only")
                ),
                SAFE_MODE
            )
            .unwrap());
        assert_eq!(image_data(&candidate), ["aW1hZ2U="]);
        assert_eq!(candidate.received_updates, 4);
    }

    #[test]
    fn content_replaces_instead_of_appending_and_empty_content_clears() {
        let mut candidate = AgentOutputCandidate::default();
        candidate
            .record_update(
                completed("image-1", vec![image("Zmlyc3Q="), image("c2Vjb25k")]),
                SAFE_MODE,
            )
            .unwrap();
        candidate
            .record_update(text("After", "after"), SAFE_MODE)
            .unwrap();
        candidate
            .record_update(
                update(
                    "image-1",
                    ToolCallUpdateFields::new().content(vec![image("cmVwbGFjZWQ=")]),
                ),
                SAFE_MODE,
            )
            .unwrap();
        assert_eq!(image_data(&candidate), ["cmVwbGFjZWQ="]);
        assert!(matches!(
            candidate.blocks()[0],
            LensOutputBlock::Image { .. }
        ));
        candidate
            .record_update(
                update("image-1", ToolCallUpdateFields::new().content(vec![])),
                SAFE_MODE,
            )
            .unwrap();
        assert!(image_data(&candidate).is_empty());
        assert_eq!(candidate.blocks().len(), 1);
    }

    #[test]
    fn interleaved_tools_keep_creation_order_and_failed_tools_have_no_visible_images() {
        let mut candidate = AgentOutputCandidate::default();
        candidate
            .record_update(
                SessionUpdate::ToolCall(ToolCall::new("first", "First")),
                SAFE_MODE,
            )
            .unwrap();
        candidate
            .record_update(completed("second", vec![image("c2Vjb25k")]), SAFE_MODE)
            .unwrap();
        candidate
            .record_update(
                update(
                    "first",
                    ToolCallUpdateFields::new()
                        .status(ToolCallStatus::Completed)
                        .content(vec![image("Zmlyc3Q=")]),
                ),
                SAFE_MODE,
            )
            .unwrap();
        assert_eq!(image_data(&candidate), ["Zmlyc3Q=", "c2Vjb25k"]);
        candidate
            .record_update(
                update(
                    "first",
                    ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
                ),
                SAFE_MODE,
            )
            .unwrap();
        assert_eq!(image_data(&candidate), ["c2Vjb25k"]);
        let mut next_turn = AgentOutputCandidate::default();
        next_turn
            .record_update(
                update(
                    "first",
                    ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
                ),
                SAFE_MODE,
            )
            .unwrap();
        assert!(!next_turn.has_output());
    }

    #[test]
    fn pending_failed_or_unknown_status_never_publishes_an_image() {
        for status in [
            ToolCallStatus::Pending,
            ToolCallStatus::InProgress,
            ToolCallStatus::Failed,
        ] {
            let mut candidate = AgentOutputCandidate::default();
            candidate
                .record_update(
                    update(
                        "image-1",
                        ToolCallUpdateFields::new()
                            .status(status)
                            .content(vec![image("aW1hZ2U=")]),
                    ),
                    SAFE_MODE,
                )
                .unwrap();
            assert!(!candidate.has_output());
            assert!(candidate.blocks().is_empty());
        }
        let mut candidate = AgentOutputCandidate::default();
        candidate
            .record_update(
                update(
                    "image-1",
                    ToolCallUpdateFields::new().content(vec![image("aW1hZ2U=")]),
                ),
                SAFE_MODE,
            )
            .unwrap();
        assert!(!candidate.has_output());
    }

    #[test]
    fn tool_images_share_message_image_validation_and_other_content_stays_private() {
        let update: SessionUpdate = serde_json::from_value(serde_json::json!({
            "sessionUpdate": "tool_call", "toolCallId": "mixed", "title": "Mixed", "status": "completed",
            "rawOutput": { "data": "aW1hZ2U=", "mimeType": "image/png" },
            "content": [
                {"type":"content", "content":{"type":"image", "mimeType":"image/png", "data":"invalid!"}},
                {"type":"content", "content":{"type":"image", "mimeType":"image/svg+xml", "data":"PHN2Zz4="}},
                {"type":"content", "content":{"type":"text", "text":"![tool prose](file:///private.png)"}},
                {"type":"content", "content":{"type":"resource_link", "name":"private", "uri":"file:///private.png"}},
                {"type":"diff", "path":"/private/file", "oldText":null, "newText":"private"},
                {"type":"terminal", "terminalId":"private-terminal"}
            ]
        })).unwrap();
        let mut candidate = AgentOutputCandidate::default();
        candidate.record_update(update, SAFE_MODE).unwrap();
        assert_eq!(candidate.blocks().len(), 2);
        assert!(candidate
            .blocks()
            .iter()
            .all(|block| matches!(block, LensOutputBlock::Unsupported { .. })));
    }

    #[test]
    fn markdown_merges_only_adjacent_content_from_the_same_message() {
        let mut candidate = AgentOutputCandidate::default();
        candidate
            .record_update(text("First ", "same"), SAFE_MODE)
            .unwrap();
        candidate
            .record_update(
                completed(
                    "text-tool",
                    vec![ContentBlock::Text(TextContent::new("Hidden")).into()],
                ),
                SAFE_MODE,
            )
            .unwrap();
        candidate
            .record_update(text("message", "same"), SAFE_MODE)
            .unwrap();
        candidate
            .record_update(completed("image", vec![image("aW1hZ2U=")]), SAFE_MODE)
            .unwrap();
        candidate
            .record_update(text("After", "same"), SAFE_MODE)
            .unwrap();
        candidate
            .record_update(text("Next", "next"), SAFE_MODE)
            .unwrap();
        assert!(matches!(candidate.blocks().as_slice(), [
            LensOutputBlock::Markdown { text: first, .. }, LensOutputBlock::Image { .. },
            LensOutputBlock::Markdown { text: after, .. }, LensOutputBlock::Markdown { text: next, .. }
        ] if first == "First message" && after == "After" && next == "Next"));
    }

    #[test]
    fn tool_state_is_bounded_and_replacement_does_not_consume_another_slot() {
        let mut candidate = AgentOutputCandidate::default();
        for index in 0..MAX_TOOL_CALLS {
            candidate
                .record_update(completed(&index.to_string(), vec![]), SAFE_MODE)
                .unwrap();
        }
        candidate
            .record_update(completed("0", vec![image("aW1hZ2U=")]), SAFE_MODE)
            .unwrap();
        assert!(candidate
            .record_update(completed("overflow", vec![]), SAFE_MODE)
            .is_err());
        assert_eq!(image_data(&candidate), ["aW1hZ2U="]);
        assert!(candidate
            .record_update(
                completed("0", vec![image("aW1hZ2U="); MAX_TOOL_IMAGE_BLOCKS + 1]),
                SAFE_MODE
            )
            .is_err());
        assert_eq!(image_data(&candidate), ["aW1hZ2U="]);
    }

    #[test]
    fn retained_image_bytes_are_bounded_even_before_tool_completion() {
        let mut candidate = AgentOutputCandidate::default();
        let data = BASE64_STANDARD.encode(vec![0_u8; MAX_INLINE_IMAGE_DECODED_BYTES]);
        for index in 0..4 {
            candidate
                .record_update(
                    update(
                        &index.to_string(),
                        ToolCallUpdateFields::new().content(vec![image(&data)]),
                    ),
                    SAFE_MODE,
                )
                .unwrap();
        }
        assert!(candidate
            .record_update(
                update(
                    "overflow",
                    ToolCallUpdateFields::new().content(vec![image(&data)])
                ),
                SAFE_MODE
            )
            .is_err());
        assert!(!candidate.has_output());
        candidate
            .record_update(
                update("0", ToolCallUpdateFields::new().content(vec![])),
                SAFE_MODE,
            )
            .unwrap();
        candidate
            .record_update(completed("replacement", vec![image(&data)]), SAFE_MODE)
            .unwrap();
        assert_eq!(image_data(&candidate).len(), 1);
    }

    #[test]
    fn safe_mode_changes_still_fail_closed() {
        let mut candidate = AgentOutputCandidate::default();
        let event: SessionUpdate = serde_json::from_value(serde_json::json!({
            "sessionUpdate":"current_mode_update", "currentModeId":"full-access"
        }))
        .unwrap();
        assert!(candidate.record_update(event, SAFE_MODE).is_err());
    }

    #[test]
    fn whitespace_only_markdown_is_not_displayable_output() {
        let mut candidate = AgentOutputCandidate::default();
        assert!(!candidate
            .record_update(text("", "same"), SAFE_MODE)
            .unwrap());
        candidate
            .record_update(text(" \n\t", "same"), SAFE_MODE)
            .unwrap();
        assert!(!candidate.has_output());
        candidate
            .record_update(completed("image", vec![image("invalid!")]), SAFE_MODE)
            .unwrap();
        assert!(candidate.has_output());
    }

    #[test]
    fn acp_text_and_image_chunks_become_typed_output_blocks() {
        let text = lens_output_block(
            ContentChunk::new(ContentBlock::Text(TextContent::new("Description")))
                .message_id("message-1"),
        );
        let image = lens_output_block(
            ContentChunk::new(ContentBlock::Image(
                ImageContent::new("iVBORw0KGgo=", "IMAGE/PNG").uri("urn:fixture:image"),
            ))
            .message_id("message-1"),
        );

        assert_eq!(
            text,
            LensOutputBlock::Markdown {
                message_id: Some("message-1".into()),
                text: "Description".into(),
            }
        );
        assert_eq!(
            image,
            LensOutputBlock::Image {
                message_id: Some("message-1".into()),
                mime_type: "image/png".into(),
                data: "iVBORw0KGgo=".into(),
                uri: Some("urn:fixture:image".into()),
            }
        );
    }

    #[test]
    fn image_output_uses_an_explicit_finite_mime_type_allowlist() {
        for mime_type in [
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "image/avif",
        ] {
            assert!(is_supported_image_mime_type(mime_type));
        }
        assert!(!is_supported_image_mime_type("image/svg+xml"));

        let unsupported = lens_output_block(ContentChunk::new(ContentBlock::Image(
            ImageContent::new("PHN2Zz4=", "image/svg+xml"),
        )));
        assert_eq!(
            unsupported,
            LensOutputBlock::Unsupported {
                message_id: None,
                content_type: "image (image/svg+xml)".into(),
            }
        );

        let invalid_data = lens_output_block(ContentChunk::new(ContentBlock::Image(
            ImageContent::new("not base64", "image/png"),
        )));
        assert_eq!(
            invalid_data,
            LensOutputBlock::Unsupported {
                message_id: None,
                content_type: "image (image/png)".into(),
            }
        );
    }

    #[test]
    fn image_output_rejects_encoded_and_decoded_payloads_above_the_inline_limit() {
        let oversized_encoded = "A".repeat(MAX_INLINE_IMAGE_ENCODED_BYTES + 1);
        assert!(!is_valid_inline_image_data(&oversized_encoded));

        let oversized_decoded =
            BASE64_STANDARD.encode(vec![0_u8; MAX_INLINE_IMAGE_DECODED_BYTES + 1]);
        assert_eq!(oversized_decoded.len(), MAX_INLINE_IMAGE_ENCODED_BYTES);
        assert!(!is_valid_inline_image_data(&oversized_decoded));

        let unsupported = lens_output_block(ContentChunk::new(ContentBlock::Image(
            ImageContent::new(oversized_encoded, "image/png"),
        )));
        assert_eq!(
            unsupported,
            LensOutputBlock::Unsupported {
                message_id: None,
                content_type: "image (image/png)".into(),
            }
        );
    }
}
