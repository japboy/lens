//! Desktop compatibility path for the shared effect-free session reducer.
pub use usecase::session_document::*;

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        ContentBlock, ContentChunk, SessionUpdate, TextContent,
    };
    #[test]
    fn append_work_scales_with_incoming_text() {
        for count in [100, 1000, 3000] {
            let mut doc = SessionDocument::default();
            let start = std::time::Instant::now();
            for _ in 0..count {
                doc.record_update(SessionUpdate::AgentMessageChunk(ContentChunk::new(
                    ContentBlock::Text(TextContent::new("x".repeat(256))),
                )))
                .unwrap();
            }
            eprintln!(
                "incremental reducer chunks={count} elapsed_ms={:.3}",
                start.elapsed().as_secs_f64() * 1000.0
            );
            assert_eq!(doc.entries.len(), 1);
        }
    }
}
