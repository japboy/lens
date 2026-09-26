//! Lightweight session transport; bodies are fetched only by the owning overlay.
use super::*;
use crate::session_document::{DocumentBlock, DocumentEntry, MessageRole};
use serde::{Deserialize, Serialize};
use usecase::response_history::LensResponseBlockDescriptor;

#[derive(Clone, Serialize)]
pub(crate) struct WireView {
    revision: u64,
    generation: Uuid,
    phase: ViewPhase,
    agent: Option<AgentKind>,
    session_id: Option<String>,
    title: Option<String>,
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    interpretation: Option<HistoryInterpretation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation: Option<Manifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch: Option<Patch>,
}

#[derive(Clone, Serialize)]
struct Manifest {
    entries: Vec<serde_json::Value>,
}
#[derive(Clone, Serialize)]
struct Patch {
    base_revision: u64,
    index: usize,
    entry: serde_json::Value,
}

#[derive(Clone, Serialize)]
struct HistoryInterpretation {
    responses: Vec<HistoryResponse>,
}

#[derive(Clone, Serialize)]
struct HistoryResponse {
    response_id: String,
    sequence: usize,
    blocks: Vec<HistoryBlock>,
}

#[derive(Clone, Serialize)]
struct HistoryBlock {
    #[serde(flatten)]
    descriptor: LensResponseBlockDescriptor,
    // Exact reference into the unchanged Conversation document, not the filtered index.
    source: serde_json::Value,
}

fn deferred_block(
    entry: &DocumentEntry,
    revision: u64,
    index: usize,
    kind: &str,
    length: usize,
) -> serde_json::Value {
    serde_json::json!({
        "type":"deferred", "entry_id":entry_id(entry), "block_index":index,
        "content_type":kind, "byte_length":length, "revision":revision,
        "append_only": matches!(entry, DocumentEntry::Message { .. }) && kind == "markdown"
    })
}

fn entry_id(entry: &DocumentEntry) -> &str {
    match entry {
        DocumentEntry::Message { id, .. } | DocumentEntry::Tool { id, .. } => id,
    }
}
fn blocks(entry: &DocumentEntry) -> &[DocumentBlock] {
    match entry {
        DocumentEntry::Message { blocks, .. } | DocumentEntry::Tool { blocks, .. } => blocks,
    }
}
fn block_kind_length(block: &DocumentBlock) -> (&'static str, usize) {
    match block {
        DocumentBlock::Markdown { text } => ("markdown", text.len()),
        DocumentBlock::Html { text } => ("html", text.len()),
        DocumentBlock::Image { data, .. } => ("image", data.len()),
        DocumentBlock::Unsupported { content_type } => ("unsupported", content_type.len()),
    }
}
fn manifest_entry(entry: &DocumentEntry, revision: u64) -> serde_json::Value {
    let id = entry_id(entry);
    let deferred = |index, kind, length| deferred_block(entry, revision, index, kind, length);
    let mut content: Vec<_> = blocks(entry)
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let (kind, length) = block_kind_length(b);
            deferred(i, kind, length)
        })
        .collect();
    match entry {
        DocumentEntry::Message { role, .. } => {
            serde_json::json!({"id":id,"kind":"message","role":role,"blocks":content})
        }
        DocumentEntry::Tool {
            title,
            status,
            accepted_html,
            ..
        } => {
            if let Some(html) = accepted_html {
                content.push(deferred(content.len(), "html", html.len()));
            }
            serde_json::json!({"id":id,"kind":"tool","title":title,"status":status,"blocks":content})
        }
    }
}

/// Replayed messages have no Lens commit receipt. Present every assistant answer
/// and completed tool artifact, grouped at user-message boundaries, without inventing
/// live run/projection provenance or copying content into the wire snapshot.
fn history_interpretation(view: &SessionView, document: &SessionDocument) -> HistoryInterpretation {
    let mut responses = Vec::new();
    let mut current: Option<HistoryResponse> = None;
    for (entry_index, entry) in document.entries.iter().enumerate() {
        if matches!(
            entry,
            DocumentEntry::Message {
                role: MessageRole::User,
                ..
            }
        ) {
            if let Some(response) = current.take() {
                responses.push(response);
            }
            continue;
        }
        let assistant = matches!(
            entry,
            DocumentEntry::Message {
                role: MessageRole::Assistant,
                ..
            }
        );
        if !assistant
            && !matches!(
                entry,
                DocumentEntry::Tool {
                    status: agent_client_protocol::schema::v1::ToolCallStatus::Completed,
                    ..
                }
            )
        {
            continue;
        }
        let revision = view
            .entry_revisions
            .get(entry_index)
            .copied()
            .unwrap_or(view.revision);
        let mut push =
            |source_index: usize, kind: &str, byte_length: usize, block: Option<&DocumentBlock>| {
                let response = current.get_or_insert_with(|| {
                    let sequence = responses.len() + 1;
                    HistoryResponse {
                        response_id: format!("history:{}:{sequence}", view.generation),
                        sequence,
                        blocks: Vec::new(),
                    }
                });
                let block_index = response.blocks.len();
                let descriptor = match block {
                    Some(DocumentBlock::Markdown { .. }) => LensResponseBlockDescriptor::Markdown {
                        block_index,
                        byte_length,
                    },
                    Some(DocumentBlock::Image { mime_type, .. }) => {
                        LensResponseBlockDescriptor::Image {
                            block_index,
                            byte_length,
                            mime_type: mime_type.clone(),
                        }
                    }
                    Some(DocumentBlock::Unsupported { content_type }) => {
                        LensResponseBlockDescriptor::Unsupported {
                            block_index,
                            content_type: content_type.clone(),
                        }
                    }
                    Some(DocumentBlock::Html { .. }) | None => LensResponseBlockDescriptor::Html {
                        block_index,
                        byte_length,
                        mime_type: "text/html".into(),
                        resource_id: format!("{}:{block_index}", response.response_id),
                        uri: format!("urn:lens:{}:{block_index}", response.response_id),
                    },
                };
                response.blocks.push(HistoryBlock {
                    descriptor,
                    source: deferred_block(entry, revision, source_index, kind, byte_length),
                });
            };
        for (source_index, block) in blocks(entry).iter().enumerate() {
            if assistant
                || matches!(
                    block,
                    DocumentBlock::Image { .. } | DocumentBlock::Html { .. }
                )
            {
                let (kind, length) = block_kind_length(block);
                push(source_index, kind, length, Some(block));
            }
        }
        if let DocumentEntry::Tool {
            accepted_html: Some(html),
            blocks,
            ..
        } = entry
        {
            push(blocks.len(), "html", html.len(), None);
        }
    }
    if let Some(response) = current {
        responses.push(response);
    }
    HistoryInterpretation { responses }
}

impl SessionView {
    pub(super) fn wire(&self, change: Option<(u64, usize)>) -> WireView {
        let entry_revision = |i: usize| {
            self.entry_revisions
                .get(i)
                .copied()
                .unwrap_or(self.revision)
        };
        let patch = change.and_then(|(base_revision, index)| {
            self.document
                .as_ref()?
                .entries
                .get(index)
                .map(|entry| Patch {
                    base_revision,
                    index,
                    entry: manifest_entry(entry, entry_revision(index)),
                })
        });
        let conversation = if patch.is_none() {
            self.document.as_ref().map(|doc| Manifest {
                entries: doc
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| manifest_entry(e, entry_revision(i)))
                    .collect(),
            })
        } else {
            None
        };
        let interpretation = if self.phase == ViewPhase::Ready {
            self.document
                .as_ref()
                .map(|document| history_interpretation(self, document))
        } else {
            None
        };
        WireView {
            revision: self.revision,
            generation: self.generation,
            phase: self.phase,
            agent: self.agent,
            session_id: self.session_id.clone(),
            title: self.title.clone(),
            error: self.error.clone(),
            interpretation,
            conversation,
            patch,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct BlockRequest {
    generation: Uuid,
    entry_id: String,
    block_index: usize,
    revision: u64,
    #[serde(default)]
    offset: usize,
}
#[derive(Clone, Serialize)]
pub(crate) struct BlockResponse {
    generation: Uuid,
    revision: u64,
    offset: usize,
    block: DocumentBlock,
}

#[tauri::command]
pub(crate) fn get_session_block<R: Runtime>(
    webview: tauri::Webview<R>,
    state: tauri::State<'_, AppState>,
    request: BlockRequest,
) -> Result<BlockResponse, String> {
    if webview.label() != crate::ui::LENS_WINDOW_LABEL {
        return Err("Session content is only available to the Lens overlay".into());
    }
    let view = state.session_view.inner.lock().map_err(lock_error)?;
    read_block(&view, request)
}

fn read_block(view: &SessionView, request: BlockRequest) -> Result<BlockResponse, String> {
    if view.generation != request.generation
        || !matches!(view.phase, ViewPhase::Live | ViewPhase::Ready)
    {
        return Err("Session content generation is stale".into());
    }
    let doc = view
        .document
        .as_ref()
        .ok_or("Session content unavailable")?;
    let (index, entry) = doc
        .entries
        .iter()
        .enumerate()
        .find(|(_, e)| entry_id(e) == request.entry_id)
        .ok_or("Session entry unavailable")?;
    let revision = view
        .entry_revisions
        .get(index)
        .copied()
        .unwrap_or(view.revision);
    if revision != request.revision {
        return Err("Session content revision is stale".into());
    }
    let block = blocks(entry).get(request.block_index);
    let mut offset = 0;
    let block = if let Some(DocumentBlock::Markdown { text }) = block {
        if matches!(entry, DocumentEntry::Message { .. })
            && request.offset <= text.len()
            && text.is_char_boundary(request.offset)
        {
            offset = request.offset;
        }
        DocumentBlock::Markdown {
            text: text[offset..].into(),
        }
    } else if let Some(block) = block {
        block.clone()
    } else if let DocumentEntry::Tool {
        accepted_html: Some(text),
        blocks,
        ..
    } = entry
    {
        if request.block_index != blocks.len() {
            return Err("Session block unavailable".into());
        }
        DocumentBlock::Html { text: text.clone() }
    } else {
        return Err("Session block unavailable".into());
    };
    Ok(BlockResponse {
        generation: view.generation,
        revision,
        offset,
        block,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::ToolCallStatus;

    fn fixture() -> SessionView {
        let mut document = SessionDocument::default();
        document.entries = vec![
            DocumentEntry::Message {
                id: "message:0".into(),
                role: MessageRole::Assistant,
                blocks: vec![DocumentBlock::Markdown {
                    text: "\u{65e5}\u{672c}\u{8a9e}🙂tail".into(),
                }],
            },
            DocumentEntry::Tool {
                id: "tool:1".into(),
                title: "Tool".into(),
                status: ToolCallStatus::Completed,
                blocks: vec![
                    DocumentBlock::Markdown {
                        text: "replacement body".into(),
                    },
                    DocumentBlock::Image {
                        mime_type: "image/png".into(),
                        data: "private-image-data".into(),
                    },
                ],
                accepted_html: Some("<style>body{color:red}</style>private HTML".into()),
            },
        ];
        SessionView {
            phase: ViewPhase::Live,
            generation: Uuid::new_v4(),
            revision: 9,
            entry_revisions: vec![4, 7],
            document: Some(document),
            ..Default::default()
        }
    }
    fn request(
        view: &SessionView,
        entry: &str,
        block_index: usize,
        revision: u64,
        offset: usize,
    ) -> BlockRequest {
        BlockRequest {
            generation: view.generation,
            entry_id: entry.into(),
            block_index,
            revision,
            offset,
        }
    }

    fn message(id: &str, role: MessageRole, text: &str) -> DocumentEntry {
        DocumentEntry::Message {
            id: id.into(),
            role,
            blocks: vec![DocumentBlock::Markdown { text: text.into() }],
        }
    }

    fn publication(id: &str, status: ToolCallStatus, html: Option<&str>) -> DocumentEntry {
        DocumentEntry::Tool {
            id: id.into(),
            title: "Synthetic publication".into(),
            status,
            blocks: vec![],
            accepted_html: html.map(str::to_owned),
        }
    }

    fn history_view(entries: Vec<DocumentEntry>) -> SessionView {
        let mut document = SessionDocument::default();
        document.entries = entries;
        SessionView {
            phase: ViewPhase::Ready,
            document: Some(document),
            ..Default::default()
        }
    }

    fn interpretation(view: &SessionView) -> serde_json::Value {
        serde_json::to_value(view.wire(None)).unwrap()["interpretation"].clone()
    }

    #[test]
    fn history_retains_all_turns_including_text_only_updates_between_media() {
        let view = history_view(vec![
            message("u0", MessageRole::User, "initial"),
            publication("p0", ToolCallStatus::Completed, Some("<p>first</p>")),
            message("a0", MessageRole::Assistant, "first narrative"),
            message("u1", MessageRole::User, "update"),
            message("a1", MessageRole::Assistant, "second text-only narrative"),
            message("u2", MessageRole::User, "update again"),
            publication("p2", ToolCallStatus::Completed, Some("<p>third</p>")),
            message("a2", MessageRole::Assistant, "third narrative"),
        ]);
        let result = interpretation(&view);
        let responses = result["responses"].as_array().unwrap();
        assert_eq!(responses.len(), 3);
        assert_eq!(
            responses
                .iter()
                .map(|r| r["sequence"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(responses[0]["blocks"].as_array().unwrap().len(), 2);
        assert_eq!(responses[1]["blocks"][0]["source"]["entry_id"], "a1");
        assert_eq!(responses[2]["blocks"][0]["source"]["entry_id"], "p2");
        assert_ne!(
            responses[0]["blocks"][0]["resource_id"],
            responses[2]["blocks"][0]["resource_id"]
        );
        assert_eq!(view.wire(None).conversation.unwrap().entries.len(), 8);
        for response in responses {
            assert!(response.get("run_id").is_none());
            assert!(response.get("projection").is_none());
        }
    }

    #[test]
    fn history_keeps_assistant_messages_without_claiming_prompt_completion() {
        for status in [
            ToolCallStatus::Pending,
            ToolCallStatus::InProgress,
            ToolCallStatus::Failed,
            ToolCallStatus::Completed,
        ] {
            let view = history_view(vec![
                message("u0", MessageRole::User, "initial"),
                publication("p0", ToolCallStatus::Completed, Some("<p>first</p>")),
                message("a0", MessageRole::Assistant, "first narrative"),
                message("u1", MessageRole::User, "update"),
                message("a1", MessageRole::Assistant, "replayed assistant text"),
                publication("p1", status, None),
                message("u2", MessageRole::User, "unanswered"),
            ]);
            let result = interpretation(&view);
            assert_eq!(result["responses"].as_array().unwrap().len(), 2);
            assert_eq!(
                result["responses"][1]["blocks"].as_array().unwrap().len(),
                1
            );
            assert_eq!(
                result["responses"][1]["blocks"][0]["source"]["entry_id"],
                "a1"
            );
        }
    }

    #[test]
    fn history_artifact_references_preserve_original_indices_and_revisions() {
        let mut view = fixture();
        view.phase = ViewPhase::Ready;
        let wire = serde_json::to_value(view.wire(None)).unwrap();
        let response = &wire["interpretation"]["responses"][0];
        assert_eq!(response["blocks"].as_array().unwrap().len(), 3);
        // Tool prose at source index 0 is not Interpretation content. The image is
        // response index 1/source index 1, and accepted HTML follows all tool blocks.
        let image = &response["blocks"][1];
        let html = &response["blocks"][2];
        assert_eq!(
            image["source"],
            wire["conversation"]["entries"][1]["blocks"][1]
        );
        assert_eq!(
            html["source"],
            wire["conversation"]["entries"][1]["blocks"][2]
        );
        assert_eq!(image["mime_type"], "image/png");
        assert_eq!(html["source"]["revision"], 7);
        let fetched = read_block(&view, request(&view, "tool:1", 2, 7, 0)).unwrap();
        assert!(
            matches!(fetched.block, DocumentBlock::Html { text } if text.contains("private HTML"))
        );
        let serialized = wire.to_string();
        assert!(wire.get("document").is_none());
        for body in ["replacement body", "private-image-data", "private HTML"] {
            assert!(!serialized.contains(body));
        }
    }

    #[test]
    fn history_excludes_user_media_and_unsuccessful_tool_artifacts() {
        for status in [
            ToolCallStatus::Pending,
            ToolCallStatus::InProgress,
            ToolCallStatus::Failed,
        ] {
            let mut user = message("u1", MessageRole::User, "question");
            if let DocumentEntry::Message { blocks, .. } = &mut user {
                blocks.push(DocumentBlock::Html {
                    text: "private user media".into(),
                });
            }
            let view = history_view(vec![
                message("a0", MessageRole::Assistant, "earlier text answer"),
                user,
                DocumentEntry::Tool {
                    id: "failed-media".into(),
                    title: "Tool".into(),
                    status,
                    blocks: vec![DocumentBlock::Image {
                        mime_type: "image/png".into(),
                        data: "c3ludGhldGlj".into(),
                    }],
                    accepted_html: Some("must not be admitted".into()),
                },
                DocumentEntry::Message {
                    id: "assistant-media".into(),
                    role: MessageRole::Assistant,
                    blocks: vec![DocumentBlock::Html {
                        text: "assistant HTML".into(),
                    }],
                },
            ]);
            let result = interpretation(&view);
            assert_eq!(result["responses"].as_array().unwrap().len(), 2);
            assert_eq!(result["responses"][0]["blocks"][0]["type"], "markdown");
            assert_eq!(
                result["responses"][1]["blocks"].as_array().unwrap().len(),
                1
            );
            assert_eq!(
                result["responses"][1]["blocks"][0]["source"]["entry_id"],
                "assistant-media"
            );
        }
    }

    #[test]
    fn history_manifest_size_does_not_scale_with_retained_body_bytes() {
        let view = history_view(vec![
            message("a0", MessageRole::Assistant, &"a".repeat(1024 * 1024)),
            message("u1", MessageRole::User, "follow-up"),
            message("a1", MessageRole::Assistant, &"b".repeat(1024 * 1024)),
        ]);
        let wire = serde_json::to_vec(&view.wire(None)).unwrap();
        assert!(wire.len() < 4096);
        assert_eq!(
            interpretation(&view)["responses"].as_array().unwrap().len(),
            2
        );
        let block = read_block(&view, request(&view, "a0", 0, view.revision, 0)).unwrap();
        assert!(
            matches!(block.block, DocumentBlock::Markdown { text } if text.len() == 1024 * 1024)
        );
        let empty = history_view(vec![message("u0", MessageRole::User, "unanswered")]);
        assert!(interpretation(&empty)["responses"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn manifest_and_patch_contain_references_without_bodies() {
        let view = fixture();
        let wire = serde_json::to_value(view.wire(None)).unwrap();
        assert!(wire.get("document").is_none());
        let serialized = wire.to_string();
        for private in [
            "\u{65e5}\u{672c}\u{8a9e}",
            "replacement body",
            "private-image-data",
            "private HTML",
        ] {
            assert!(!serialized.contains(private));
        }
        assert_eq!(
            wire["conversation"]["entries"][0]["blocks"][0]["byte_length"],
            "\u{65e5}\u{672c}\u{8a9e}🙂tail".len()
        );
        assert_eq!(
            wire["conversation"]["entries"][0]["blocks"][0]["revision"],
            4
        );
        assert_eq!(
            wire["conversation"]["entries"][0]["blocks"][0]["append_only"],
            true
        );
        assert_eq!(
            wire["conversation"]["entries"][1]["blocks"][2]["content_type"],
            "html"
        );
        assert_eq!(
            wire["conversation"]["entries"][1]["blocks"][0]["append_only"],
            false
        );
        let patch = serde_json::to_value(view.wire(Some((8, 1)))).unwrap();
        assert!(patch.get("conversation").is_none());
        assert_eq!(patch["patch"]["base_revision"], 8);
        assert_eq!(patch["patch"]["index"], 1);
        assert_eq!(patch["patch"]["entry"]["blocks"][0]["revision"], 7);
        assert!(!patch.to_string().contains("replacement body"));
    }

    #[test]
    fn message_suffix_uses_utf8_bytes_and_invalid_offsets_return_full_body() {
        let view = fixture();
        let full = "\u{65e5}\u{672c}\u{8a9e}🙂tail";
        for (requested, actual) in [
            (
                "\u{65e5}\u{672c}\u{8a9e}".len(),
                "\u{65e5}\u{672c}\u{8a9e}".len(),
            ),
            (full.len(), full.len()),
            (1, 0),
            (usize::MAX, 0),
        ] {
            let result = read_block(&view, request(&view, "message:0", 0, 4, requested)).unwrap();
            assert_eq!(result.offset, actual);
            assert_eq!(result.revision, 4);
            assert_eq!(
                result.block,
                DocumentBlock::Markdown {
                    text: full[actual..].into()
                }
            );
        }
    }

    #[test]
    fn stale_generation_revision_and_non_readable_phases_are_rejected() {
        let mut view = fixture();
        let mut stale = request(&view, "message:0", 0, 4, 0);
        stale.generation = Uuid::new_v4();
        assert!(read_block(&view, stale)
            .err()
            .unwrap()
            .contains("generation"));
        for revision in [3, 5, 9] {
            assert!(
                read_block(&view, request(&view, "message:0", 0, revision, 0))
                    .err()
                    .unwrap()
                    .contains("revision")
            );
        }
        for phase in [ViewPhase::Idle, ViewPhase::Loading, ViewPhase::Failed] {
            view.phase = phase;
            assert!(read_block(&view, request(&view, "message:0", 0, 4, 0))
                .err()
                .unwrap()
                .contains("generation"));
        }
        view.phase = ViewPhase::Ready;
        assert!(read_block(&view, request(&view, "message:0", 0, 4, 0)).is_ok());
    }

    #[test]
    fn tool_replacements_and_accepted_html_always_return_complete_content() {
        let view = fixture();
        let result = read_block(&view, request(&view, "tool:1", 0, 7, 5)).unwrap();
        assert_eq!(result.offset, 0);
        assert_eq!(
            result.block,
            DocumentBlock::Markdown {
                text: "replacement body".into()
            }
        );
        let html = read_block(&view, request(&view, "tool:1", 2, 7, 20)).unwrap();
        assert_eq!(html.offset, 0);
        assert!(
            matches!(html.block, DocumentBlock::Html { text } if text.contains("private HTML"))
        );
        assert!(read_block(&view, request(&view, "tool:1", 3, 7, 0)).is_err());
        assert!(read_block(&view, request(&view, "missing", 0, 7, 0)).is_err());
    }
}
