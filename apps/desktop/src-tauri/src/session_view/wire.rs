//! Lightweight session transport; bodies are fetched only by the owning overlay.
use super::*;
use crate::session_document::{DocumentBlock, DocumentEntry, MessageRole};
use serde::{Deserialize, Serialize};

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
    document: Option<SessionDocument>,
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
    let deferred = |index, kind, length| {
        serde_json::json!({
            "type":"deferred", "entry_id":id, "block_index":index,
            "content_type":kind, "byte_length":length, "revision":revision,
            "append_only": matches!(entry, DocumentEntry::Message { .. }) && kind == "markdown"
        })
    };
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

/// Select the update containing the latest successful media result. A replay does
/// not attest prompt completion: later prompts, progress, or unsuccessful calls
/// must not hide that result. Text-only histories keep their last-answer behavior.
fn history_answer_entries(document: &SessionDocument) -> &[DocumentEntry] {
    let is_user = |entry: &DocumentEntry| {
        matches!(
            entry,
            DocumentEntry::Message {
                role: MessageRole::User,
                ..
            }
        )
    };
    let latest_media = document.entries.iter().rposition(|entry| {
        matches!(entry, DocumentEntry::Tool { status, blocks, accepted_html, .. }
            if *status == agent_client_protocol::schema::v1::ToolCallStatus::Completed
                && (accepted_html.is_some()
                    || blocks.iter().any(|block| matches!(block,
                        DocumentBlock::Html { .. } | DocumentBlock::Image { .. }))))
    });
    let end = latest_media
        .and_then(|index| {
            document.entries[index + 1..]
                .iter()
                .position(is_user)
                .map(|offset| index + 1 + offset)
        })
        .unwrap_or(document.entries.len());
    let start = document.entries[..end]
        .iter()
        .rposition(is_user)
        .map_or(0, |index| index + 1);
    &document.entries[start..end]
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
        // Interpretation receives one selected update; Conversation retains every entry.
        let document = if self.phase == ViewPhase::Ready {
            self.document.as_ref().map(|doc| {
                let mut projection = SessionDocument::default();
                projection.entries = history_answer_entries(doc)
                    .iter()
                    .filter_map(|entry| match entry {
                        DocumentEntry::Message {
                            role: MessageRole::Assistant,
                            ..
                        } => Some(entry.clone()),
                        DocumentEntry::Tool {
                            id,
                            title,
                            status,
                            blocks,
                            accepted_html,
                        } if *status
                            == agent_client_protocol::schema::v1::ToolCallStatus::Completed =>
                        {
                            Some(DocumentEntry::Tool {
                                id: id.clone(),
                                title: title.clone(),
                                status: *status,
                                blocks: blocks
                                    .iter()
                                    .filter(|block| {
                                        matches!(
                                            block,
                                            DocumentBlock::Html { .. }
                                                | DocumentBlock::Image { .. }
                                        )
                                    })
                                    .cloned()
                                    .collect(),
                                accepted_html: accepted_html.clone(),
                            })
                        }
                        _ => None,
                    })
                    .collect();
                projection
            })
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
            document,
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

    #[test]
    fn history_retains_successful_output_across_unpublished_updates() {
        let original = vec![
            message("u0", MessageRole::User, "initial"),
            publication("p0", ToolCallStatus::Completed, Some("<p>first</p>")),
            message("a0", MessageRole::Assistant, "first narrative"),
        ];
        let expected = history_view(original.clone())
            .wire(None)
            .document
            .unwrap()
            .entries;
        for status in [
            None,
            Some(ToolCallStatus::Pending),
            Some(ToolCallStatus::InProgress),
            Some(ToolCallStatus::Failed),
            Some(ToolCallStatus::Completed),
        ] {
            let mut entries = original.clone();
            entries.push(message("u1", MessageRole::User, "update"));
            entries.push(message("a1", MessageRole::Assistant, "working on update"));
            if let Some(status) = status {
                entries.push(publication("p1", status, None));
            }
            entries.push(message("u2", MessageRole::User, "another update"));
            let count = entries.len();
            let wire = history_view(entries).wire(None);
            assert_eq!(wire.document.unwrap().entries, expected);
            assert_eq!(wire.conversation.unwrap().entries.len(), count);
        }
    }

    #[test]
    fn history_selects_new_success_without_mixing_update_narratives() {
        let wire = history_view(vec![
            message("u0", MessageRole::User, "initial"),
            publication("p0", ToolCallStatus::Completed, Some("<p>first</p>")),
            message("a0", MessageRole::Assistant, "first narrative"),
            message("u1", MessageRole::User, "update"),
            publication("p1", ToolCallStatus::Completed, Some("<p>second</p>")),
            message("a1", MessageRole::Assistant, "second narrative"),
            publication("pending-after-success", ToolCallStatus::Pending, None),
            message("u2", MessageRole::User, "pending"),
        ])
        .wire(None);
        assert_eq!(
            wire.document.unwrap().entries,
            vec![
                publication("p1", ToolCallStatus::Completed, Some("<p>second</p>")),
                message("a1", MessageRole::Assistant, "second narrative"),
            ]
        );
    }

    #[test]
    fn history_retains_legacy_media_and_preserves_assistant_only_selection() {
        for block in [
            DocumentBlock::Html {
                text: "<p>legacy</p>".into(),
            },
            DocumentBlock::Image {
                mime_type: "image/png".into(),
                data: "synthetic".into(),
            },
        ] {
            let output = DocumentEntry::Tool {
                id: "media".into(),
                title: "media".into(),
                status: ToolCallStatus::Completed,
                blocks: vec![block],
                accepted_html: None,
            };
            let wire = history_view(vec![
                output.clone(),
                message("u", MessageRole::User, "pending"),
            ])
            .wire(None);
            assert_eq!(wire.document.unwrap().entries, vec![output]);
        }
        let answer = message("a", MessageRole::Assistant, "text answer");
        let wire = history_view(vec![
            answer.clone(),
            message("u", MessageRole::User, "pending"),
        ])
        .wire(None);
        assert!(wire.document.unwrap().entries.is_empty());
        let latest = message("latest", MessageRole::Assistant, "new text answer");
        assert_eq!(
            history_view(vec![
                answer,
                message("u", MessageRole::User, "question"),
                latest.clone()
            ])
            .wire(None)
            .document
            .unwrap()
            .entries,
            vec![latest]
        );
        assert!(
            history_view(vec![message("u", MessageRole::User, "unanswered")])
                .wire(None)
                .document
                .unwrap()
                .entries
                .is_empty()
        );
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
