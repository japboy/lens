//! Operation-owned immutable responses. Bodies are shared privately; snapshots carry manifests.
use crate::{
    live_sync::ProjectionRef,
    model::{LensOutputBlock, LensRepresentation},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

pub const MAX_RESPONSE_HISTORY_ENTRIES: usize = 128;
pub const MAX_RESPONSE_HISTORY_BYTES: usize = 64 * 1024 * 1024;
pub const RESPONSE_HISTORY_CAPACITY_MESSAGE: &str =
    "Response history is full. Existing responses are retained. Stop Lens and start a new session to continue.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LensResponseBlockDescriptor {
    Markdown {
        block_index: usize,
        byte_length: usize,
    },
    Image {
        block_index: usize,
        mime_type: String,
        byte_length: usize,
    },
    Html {
        block_index: usize,
        mime_type: String,
        resource_id: String,
        uri: String,
        byte_length: usize,
    },
    Unsupported {
        block_index: usize,
        content_type: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LensResponseManifest {
    pub sequence: usize,
    pub representation_id: Uuid,
    pub run_id: Uuid,
    pub acp_session_id: Option<String>,
    pub prompt_execution_revision: u32,
    pub context_id: Uuid,
    pub context_revision: u64,
    pub projection: ProjectionRef,
    pub block_count: usize,
    pub retained_bytes: usize,
    pub blocks: Vec<LensResponseBlockDescriptor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LensResponseHistory {
    pub responses: Vec<LensResponseManifest>,
    pub retained_bytes: usize,
    pub capacity_reached: bool,
    // Cloning AppSnapshot copies Arc handles, never accumulated output bodies.
    #[serde(skip)]
    bodies: Vec<Arc<LensRepresentation>>,
}

impl LensResponseHistory {
    pub fn contains_run(&self, run_id: Uuid) -> bool {
        self.responses
            .iter()
            .any(|response| response.run_id == run_id)
    }

    pub fn representation(&self, id: Uuid) -> Option<&LensRepresentation> {
        self.bodies
            .iter()
            .find(|response| response.representation_id == id)
            .map(Arc::as_ref)
    }

    /// Called under the host's publication lock, after its existing candidate guards.
    /// A duplicate is idempotent; capacity rejection never evicts or partially appends.
    pub fn append(
        &mut self,
        representation: LensRepresentation,
        session_id: Option<String>,
    ) -> Result<bool, &'static str> {
        self.append_with_limits(
            representation,
            session_id,
            MAX_RESPONSE_HISTORY_ENTRIES,
            MAX_RESPONSE_HISTORY_BYTES,
        )
    }

    fn append_with_limits(
        &mut self,
        representation: LensRepresentation,
        session_id: Option<String>,
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<bool, &'static str> {
        if self.contains_run(representation.run_id) {
            return Ok(false);
        }
        let blocks = representation
            .output_blocks
            .iter()
            .enumerate()
            .map(|(block_index, block)| match block {
                LensOutputBlock::Markdown { text, .. } => LensResponseBlockDescriptor::Markdown {
                    block_index,
                    byte_length: text.len(),
                },
                LensOutputBlock::Image {
                    mime_type, data, ..
                } => LensResponseBlockDescriptor::Image {
                    block_index,
                    mime_type: mime_type.clone(),
                    byte_length: data.len(),
                },
                LensOutputBlock::Html {
                    resource_id,
                    mime_type,
                    uri,
                    text,
                    ..
                } => LensResponseBlockDescriptor::Html {
                    block_index,
                    mime_type: mime_type.clone(),
                    resource_id: resource_id.clone(),
                    uri: uri.clone(),
                    byte_length: text.len(),
                },
                LensOutputBlock::Unsupported { content_type, .. } => {
                    LensResponseBlockDescriptor::Unsupported {
                        block_index,
                        content_type: content_type.clone(),
                    }
                }
            })
            .collect::<Vec<_>>();
        // Charge every retained string (including private HTML and image base64), plus
        // representation/manifest metadata. This is a payload budget, not an RSS promise.
        let mut manifest = LensResponseManifest {
            sequence: self.responses.len() + 1,
            representation_id: representation.representation_id,
            run_id: representation.run_id,
            acp_session_id: session_id,
            prompt_execution_revision: representation.prompt_execution_revision,
            context_id: representation.context_id,
            context_revision: representation.context_revision,
            projection: representation.projection.clone(),
            block_count: blocks.len(),
            retained_bytes: 0,
            blocks,
        };
        let body_bytes = representation
            .output_blocks
            .iter()
            .fold(0usize, |sum, block| sum.saturating_add(block_bytes(block)));
        let metadata_bytes = serde_json::to_vec(&manifest)
            .map_err(|_| "Response metadata could not be serialized")?
            .len();
        let bytes = body_bytes
            .saturating_add(metadata_bytes)
            .saturating_add(std::mem::size_of::<LensRepresentation>());
        let next_bytes = self.retained_bytes.checked_add(bytes);
        if self.capacity_reached
            || self.responses.len() >= max_entries
            || next_bytes.is_none_or(|n| n > max_bytes)
        {
            self.capacity_reached = true;
            return Err(RESPONSE_HISTORY_CAPACITY_MESSAGE);
        }
        manifest.retained_bytes = bytes;
        self.bodies.push(Arc::new(representation));
        self.responses.push(manifest);
        self.retained_bytes = next_bytes.expect("checked budget");
        self.capacity_reached =
            self.responses.len() == max_entries || self.retained_bytes == max_bytes;
        Ok(true)
    }
}

fn block_bytes(block: &LensOutputBlock) -> usize {
    let (message, fields): (&Option<String>, Vec<&str>) = match block {
        LensOutputBlock::Markdown { message_id, text } => (message_id, vec![text]),
        LensOutputBlock::Image {
            message_id,
            mime_type,
            data,
            uri,
        } => (
            message_id,
            vec![mime_type, data, uri.as_deref().unwrap_or("")],
        ),
        LensOutputBlock::Html {
            message_id,
            resource_id,
            mime_type,
            uri,
            text,
            ..
        } => (message_id, vec![resource_id, mime_type, uri, text]),
        LensOutputBlock::Unsupported {
            message_id,
            content_type,
        } => (message_id, vec![content_type]),
    };
    fields.iter().fold(
        std::mem::size_of::<LensOutputBlock>()
            .saturating_add(message.as_ref().map_or(0, String::len)),
        |sum, text| sum.saturating_add(text.len()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(run: u128, text: &str) -> LensRepresentation {
        LensRepresentation {
            prompt_execution_revision: 1,
            representation_id: Uuid::from_u128(run + 100),
            context_id: Uuid::from_u128(1),
            context_revision: 1,
            projection: ProjectionRef::new(
                std::num::NonZeroU64::new(1).unwrap(),
                "0".repeat(64).parse().unwrap(),
            ),
            run_id: Uuid::from_u128(run),
            output_blocks: vec![LensOutputBlock::Markdown {
                message_id: Some("same-message".into()),
                text: text.into(),
            }]
            .into(),
        }
    }

    #[test]
    fn ordered_history_shares_bodies_and_serializes_only_descriptors() {
        let mut history = LensResponseHistory::default();
        let first = response(1, "private-first-markdown");
        let latest = response(2, "private-second-markdown");
        history
            .append(first.clone(), Some("session-a".into()))
            .unwrap();
        history
            .append(latest.clone(), Some("session-b".into()))
            .unwrap();
        let cloned = history.clone();
        assert!(Arc::ptr_eq(
            &first.output_blocks,
            &cloned
                .representation(first.representation_id)
                .unwrap()
                .output_blocks
        ));
        assert!(Arc::ptr_eq(&history.bodies[0], &cloned.bodies[0]));
        assert_eq!(
            history
                .responses
                .iter()
                .map(|r| r.sequence)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let json = serde_json::to_string(&history).unwrap();
        assert!(!json.contains("private-first-markdown"));
        assert!(!json.contains("private-second-markdown"));
        let wire: LensResponseHistory = serde_json::from_str(&json).unwrap();
        assert_eq!(wire.responses, history.responses);
        assert!(
            wire.bodies.is_empty(),
            "IPC is not authoritative body persistence"
        );
    }

    #[test]
    fn run_identity_is_idempotent_and_distinct_runs_on_equal_projection_append() {
        let mut history = LensResponseHistory::default();
        let first = response(1, "first");
        assert_eq!(history.append(first.clone(), None), Ok(true));
        let before = history.clone();
        assert_eq!(history.append(first, None), Ok(false));
        assert_eq!(history, before);
        assert_eq!(history.append(response(2, "retry"), None), Ok(true));
        assert_eq!(history.responses.len(), 2);
    }

    #[test]
    fn count_and_byte_capacity_preserve_all_existing_entries() {
        for by_count in [true, false] {
            let mut history = LensResponseHistory::default();
            history.append(response(1, "first"), None).unwrap();
            let before = history.clone();
            let max_count = if by_count { 1 } else { 128 };
            let max_bytes = if by_count {
                MAX_RESPONSE_HISTORY_BYTES
            } else {
                history.retained_bytes + 1
            };
            assert_eq!(
                history.append_with_limits(response(2, "second"), None, max_count, max_bytes),
                Err(RESPONSE_HISTORY_CAPACITY_MESSAGE)
            );
            assert_eq!(history.responses, before.responses);
            assert_eq!(history.bodies, before.bodies);
            assert_eq!(history.retained_bytes, before.retained_bytes);
            assert!(history.capacity_reached);
        }
    }

    #[test]
    fn html_and_image_bytes_are_charged_but_absent_from_manifest() {
        let mut representation = response(1, "hello");
        representation.output_blocks = vec![
            LensOutputBlock::Image {
                message_id: None,
                mime_type: "image/png".into(),
                data: "aW1hZ2U=".repeat(100),
                uri: None,
            },
            LensOutputBlock::Html {
                message_id: None,
                mime_type: "text/html".into(),
                resource_id: "html-id".into(),
                uri: "urn:html".into(),
                byte_length: 1000,
                text: "h".repeat(1000),
            },
        ]
        .into();
        let mut history = LensResponseHistory::default();
        history.append(representation, None).unwrap();
        assert!(history.retained_bytes >= 1800);
        let json = serde_json::to_value(&history).unwrap();
        assert_eq!(json["responses"][0]["blocks"][0]["byte_length"], 800);
        assert_eq!(json["responses"][0]["blocks"][1]["byte_length"], 1000);
        assert!(json["responses"][0]["blocks"][0].get("data").is_none());
        assert!(json["responses"][0]["blocks"][1].get("text").is_none());
    }
}
