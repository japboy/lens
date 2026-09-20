//! Ordered, bounded metadata persistence outside the live ACP dispatch path.
use crate::session_history_store::{HistorySource, HistoryStore, InfoPatch, Patch, StoredEntry};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
const QUEUE_CAPACITY: usize = 128;
const MAX_FIELD_BYTES: usize = 65_536;
enum Operation {
    RecordLocal {
        source: HistorySource,
        entry: StoredEntry,
    },
    Patch {
        source: HistorySource,
        id: String,
        cwd: String,
        patch: InfoPatch,
    },
    Barrier(mpsc::SyncSender<()>),
}
pub(crate) struct HistoryWriter {
    sender: Option<mpsc::SyncSender<Operation>>,
    worker: Option<JoinHandle<()>>,
    last_error: Arc<Mutex<Option<String>>>,
}
impl HistoryWriter {
    pub fn new(store: Arc<HistoryStore>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let last_error = Arc::new(Mutex::new(None));
        let errors = Arc::clone(&last_error);
        let worker = thread::Builder::new()
            .name("lens-history-writer".into())
            .spawn(move || {
                while let Ok(operation) = receiver.recv() {
                    let result = match operation {
                        Operation::RecordLocal { source, entry } => {
                            store.record_local(&source, &entry)
                        }
                        Operation::Patch {
                            source,
                            id,
                            cwd,
                            patch,
                        } => store
                            .apply_info_patch(&source, &id, &cwd, patch)
                            .map(|_| ()),
                        Operation::Barrier(done) => {
                            let _ = done.send(());
                            Ok(())
                        }
                    };
                    if let Err(error) = result {
                        if let Ok(mut notice) = errors.lock() {
                            *notice = Some(error);
                        }
                    }
                }
            })
            .map_err(|error| format!("Unable to start history writer: {error}"))?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
            last_error,
        })
    }
    pub fn record_local(&self, source: HistorySource, entry: StoredEntry) -> Result<(), String> {
        check_lengths(&[
            source.invocation.len(),
            entry.session_id.len(),
            entry.cwd.len(),
            entry.title.as_ref().map_or(0, String::len),
            entry.provider_updated_at.as_ref().map_or(0, String::len),
            entry.local_activity_at.as_ref().map_or(0, String::len),
        ])?;
        self.try_enqueue(Operation::RecordLocal { source, entry })
    }
    pub fn apply_info_patch(
        &self,
        source: HistorySource,
        id: String,
        cwd: String,
        patch: InfoPatch,
    ) -> Result<(), String> {
        let value = |patch: &Patch<String>| match patch {
            Patch::Value(value) => Some(value.len()),
            _ => None,
        };
        check_lengths(&[
            source.invocation.len(),
            id.len(),
            cwd.len(),
            value(&patch.title).unwrap_or(0),
            value(&patch.provider_updated_at).unwrap_or(0),
        ])?;
        self.try_enqueue(Operation::Patch {
            source,
            id,
            cwd,
            patch,
        })
    }
    fn try_enqueue(&self, operation: Operation) -> Result<(), String> {
        self.sender
            .as_ref()
            .ok_or("History writer is closed")?
            .try_send(operation)
            .map_err(|error| {
                let message = match error {
                    mpsc::TrySendError::Full(_) => {
                        "History persistence queue is full; a metadata update was not saved"
                    }
                    mpsc::TrySendError::Disconnected(_) => {
                        "History persistence worker is unavailable"
                    }
                }
                .to_string();
                // Reporting a full queue must not make the ACP caller wait on the worker.
                if let Ok(mut notice) = self.last_error.try_lock() {
                    *notice = Some(message.clone());
                }
                message
            })
    }
    /// Blocking barrier: call from spawn_blocking, never from live ACP dispatch.
    /// Earlier write errors remain available through notice(), independently of this barrier.
    pub fn flush(&self) -> Result<(), String> {
        let (done, completion) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .ok_or("History writer is closed")?
            .send(Operation::Barrier(done))
            .map_err(|_| "History persistence worker is unavailable")?;
        completion
            .recv()
            .map_err(|_| "History persistence worker stopped before completing writes".into())
    }
    pub fn notice(&self) -> Option<String> {
        self.last_error
            .lock()
            .ok()
            .and_then(|notice| notice.clone())
    }
}
impl Drop for HistoryWriter {
    fn drop(&mut self) {
        // No worker-owned sender or self Arc: disconnect drains all accepted writes.
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn check_lengths(lengths: &[usize]) -> Result<(), String> {
    if lengths.iter().any(|length| *length > MAX_FIELD_BYTES) {
        Err("History metadata exceeds the field size limit".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::AgentKind, session_history_store::EntryState};
    fn source() -> HistorySource {
        HistorySource {
            agent: AgentKind::Codex,
            invocation: "test".into(),
        }
    }
    fn local(id: &str) -> StoredEntry {
        StoredEntry {
            session_id: id.into(),
            cwd: "/work".into(),
            title: Some("local".into()),
            provider_updated_at: None,
            local_activity_at: Some("2026-09-20T00:00:00Z".into()),
            state: EntryState::LocalOnly,
            can_load: None,
        }
    }
    #[test]
    fn writes_are_ordered_nonfatal_and_drained_on_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = Arc::new(HistoryStore::open(&path).unwrap());
        let writer = HistoryWriter::new(Arc::clone(&store)).unwrap();
        writer.record_local(source(), local("")).unwrap();
        writer.record_local(source(), local("one")).unwrap();
        let patch = InfoPatch {
            title: Patch::Value("provider".into()),
            ..Default::default()
        };
        writer
            .apply_info_patch(source(), "one".into(), "/work".into(), patch)
            .unwrap();
        writer.record_local(source(), local("one")).unwrap();
        writer.flush().unwrap();
        let rows = store.entries(&source(), "/work").unwrap();
        assert_eq!(rows[0].title.as_deref(), Some("provider"));
        assert!(writer.notice().is_some());
        writer.record_local(source(), local("at-shutdown")).unwrap();
        drop(writer);
        drop(store);
        let reopened = HistoryStore::open(&path).unwrap();
        assert_eq!(reopened.entries(&source(), "/work").unwrap().len(), 2);
    }
    #[test]
    fn database_write_failure_is_reported_without_stopping_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = Arc::new(HistoryStore::open(&path).unwrap());
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_history BEFORE INSERT ON history_entries BEGIN SELECT RAISE(ABORT, 'history unavailable'); END;").unwrap();
        let writer = HistoryWriter::new(Arc::clone(&store)).unwrap();
        writer.record_local(source(), local("rejected")).unwrap();
        writer.flush().unwrap();
        assert!(writer.notice().unwrap().contains("history unavailable"));
        assert!(store.entries(&source(), "/work").unwrap().is_empty());
        connection
            .execute_batch("DROP TRIGGER reject_history;")
            .unwrap();
        writer.record_local(source(), local("recovered")).unwrap();
        writer.flush().unwrap();
        assert_eq!(
            store.entries(&source(), "/work").unwrap()[0].session_id,
            "recovered"
        );
    }

    #[tokio::test]
    async fn rejected_metadata_does_not_interrupt_live_notifications() {
        use crate::{app_state::AppState, model::LensStage, session_view, test_support};
        use agent_client_protocol::schema::v1::{
            ContentBlock, ContentChunk, SessionInfoUpdate, SessionUpdate, TextContent,
        };
        use tauri::Manager;
        let state = test_support::state();
        let operation = uuid::Uuid::new_v4();
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.lens.operation_id = Some(operation);
            snapshot.lens.stage = LensStage::Extracting;
        }
        let app = crate::configure_shell(
            tauri::test::mock_builder().manage(state),
            crate::platform::Presentation(Arc::new(test_support::UnusedPresentation)),
            crate::ui::TrayPresentation(Arc::new(test_support::UnusedTray)),
            crate::agent::AgentServices(Arc::new(test_support::UnusedAgent)),
        )
        .build(crate::product_context())
        .unwrap();
        session_view::start_live(app.handle(), operation, AgentKind::Codex, "live".into()).unwrap();
        session_view::record_live(
            app.handle(),
            "live",
            SessionUpdate::SessionInfoUpdate(
                SessionInfoUpdate::new().title("x".repeat(MAX_FIELD_BYTES + 1)),
            ),
        )
        .unwrap();
        session_view::record_live(
            app.handle(),
            "live",
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("still running"),
            ))),
        )
        .unwrap();
        let view = app.state::<AppState>().session_view.view().unwrap();
        assert_eq!(view.phase, session_view::ViewPhase::Live);
        let json = serde_json::to_value(view.document.unwrap()).unwrap();
        assert!(json.to_string().contains("still running"));
    }
}
