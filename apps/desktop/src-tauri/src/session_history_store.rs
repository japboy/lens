//! Durable metadata only. Reading this store never connects to an Agent.
use crate::model::AgentKind;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::{collections::HashSet, path::Path, sync::Mutex, time::Duration};

const MAX_ENTRIES: usize = 10_000;
const MAX_FIELD_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistorySource {
    pub agent: AgentKind,
    pub invocation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryState {
    Listed,
    LocalOnly,
    NotSeen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredEntry {
    pub session_id: String,
    pub cwd: String,
    pub title: Option<String>,
    pub provider_updated_at: Option<String>,
    pub local_activity_at: Option<String>,
    pub state: EntryState,
    pub can_load: Option<bool>,
}

#[derive(Debug, Clone)]
pub(crate) struct ListedEntry {
    pub session_id: String,
    pub title: Option<String>,
    pub provider_updated_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) enum Patch<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InfoPatch {
    pub title: Patch<String>,
    pub provider_updated_at: Patch<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanToken {
    pub source: HistorySource,
    pub cwd: String,
    revision: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ScanOutcome {
    #[default]
    Never,
    Complete,
    Partial,
    Failed,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ScanSummary {
    pub checked_at: Option<String>,
    pub outcome: ScanOutcome,
    pub notice: Option<String>,
}

pub(crate) struct HistoryStore {
    connection: Mutex<Connection>,
}

fn error(value: impl std::fmt::Display) -> String {
    format!("Session history storage: {value}")
}
fn agent_key(source: &HistorySource) -> Result<String, String> {
    serde_json::to_string(&source.agent).map_err(error)
}
fn field(value: &str) -> Result<(), String> {
    if value.len() > MAX_FIELD_BYTES || value.contains('\0') {
        return Err(error("metadata field exceeds capacity or contains NUL"));
    }
    Ok(())
}
fn scope(source: &HistorySource, cwd: &str) -> Result<(), String> {
    field(&source.invocation)?;
    field(cwd)?;
    if source.invocation.is_empty() || !Path::new(cwd).is_absolute() {
        return Err(error("invalid connection identity or working directory"));
    }
    Ok(())
}
fn session_id(id: &str) -> Result<(), String> {
    field(id)?;
    if id.is_empty() {
        return Err(error("empty session ID"));
    }
    Ok(())
}
fn optional(value: &Option<String>) -> Result<(), String> {
    if let Some(value) = value {
        field(value)?;
    }
    Ok(())
}
fn revision(tx: &Transaction<'_>) -> rusqlite::Result<i64> {
    tx.query_row(
        "UPDATE history_clock SET revision = revision + 1 WHERE id = 1 RETURNING revision",
        [],
        |row| row.get(0),
    )
}
fn current(tx: &Transaction<'_>, token: &ScanToken) -> Result<bool, String> {
    let value: Option<i64> = tx
        .query_row(
            "SELECT revision FROM history_scans WHERE agent = ?1 AND invocation = ?2 AND cwd = ?3",
            params![
                agent_key(&token.source)?,
                token.source.invocation,
                token.cwd
            ],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?;
    Ok(value == Some(token.revision))
}
fn capacity(tx: &Transaction<'_>, source: &HistorySource, cwd: &str) -> Result<(), String> {
    let count: i64 = tx.query_row("SELECT count(*) FROM history_entries WHERE agent = ?1 AND invocation = ?2 AND cwd = ?3",
        params![agent_key(source)?, source.invocation, cwd], |r| r.get(0)).map_err(error)?;
    if count > MAX_ENTRIES as i64 {
        return Err(error(
            "metadata capacity exceeded for this Agent and directory",
        ));
    }
    Ok(())
}

impl HistoryStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(error)?;
        }
        Self::initialize(Connection::open(path).map_err(error)?)
    }

    #[cfg(test)]
    pub fn memory() -> Result<Self, String> {
        Self::initialize(Connection::open_in_memory().map_err(error)?)
    }

    fn initialize(mut connection: Connection) -> Result<Self, String> {
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(error)?;
        let tx = connection.transaction().map_err(error)?;
        let version: i64 = tx
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(error)?;
        match version {
            0 => tx.execute_batch(
                "CREATE TABLE history_clock (id INTEGER PRIMARY KEY CHECK(id = 1), revision INTEGER NOT NULL);
                 INSERT INTO history_clock VALUES (1, 0);
                 CREATE TABLE history_scans (
                   agent TEXT NOT NULL, invocation TEXT NOT NULL, cwd TEXT NOT NULL, revision INTEGER NOT NULL,
                   checked_at TEXT, outcome INTEGER NOT NULL DEFAULT 0 CHECK(outcome IN (0,1,2,3)), notice TEXT,
                   PRIMARY KEY(agent, invocation, cwd));
                 CREATE TABLE history_entries (
                   agent TEXT NOT NULL, invocation TEXT NOT NULL, cwd TEXT NOT NULL, session_id TEXT NOT NULL,
                   title TEXT, provider_updated_at TEXT, local_activity_at TEXT,
                   state INTEGER NOT NULL CHECK(state IN (0,1,2)), can_load INTEGER CHECK(can_load IN (0,1)),
                   revision INTEGER NOT NULL,
                   PRIMARY KEY(agent, invocation, cwd, session_id));
                 PRAGMA user_version = 1;"
            ).map_err(error)?,
            1 => {
                // Validate the expected schema before exposing a supposedly usable cache.
                tx.prepare("SELECT title, provider_updated_at, local_activity_at, state, can_load, revision FROM history_entries LIMIT 0").map_err(error)?;
                tx.prepare("SELECT revision,checked_at,outcome,notice FROM history_scans LIMIT 0").map_err(error)?;
                tx.query_row("SELECT revision FROM history_clock WHERE id = 1", [], |r| r.get::<_, i64>(0)).map_err(error)?;
            }
            _ => return Err(error(format!("unsupported schema version {version}"))),
        }
        tx.commit().map_err(error)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn begin_scan(&self, source: &HistorySource, cwd: &str) -> Result<ScanToken, String> {
        scope(source, cwd)?;
        let mut connection = self.connection.lock().map_err(error)?;
        let tx = connection.transaction().map_err(error)?;
        let revision = revision(&tx).map_err(error)?;
        tx.execute("INSERT INTO history_scans (agent, invocation, cwd, revision) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(agent, invocation, cwd) DO UPDATE SET revision=excluded.revision",
            params![agent_key(source)?, source.invocation, cwd, revision]).map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(ScanToken {
            source: source.clone(),
            cwd: cwd.into(),
            revision,
        })
    }

    pub fn finish_scan(
        &self,
        token: &ScanToken,
        checked_at: &str,
        outcome: ScanOutcome,
        notice: Option<&str>,
    ) -> Result<bool, String> {
        field(checked_at)?;
        if let Some(notice) = notice {
            field(notice)?;
        }
        let outcome = match outcome {
            ScanOutcome::Never => return Err(error("a completed scan must have an outcome")),
            ScanOutcome::Complete => 1,
            ScanOutcome::Partial => 2,
            ScanOutcome::Failed => 3,
        };
        let mut connection = self.connection.lock().map_err(error)?;
        let tx = connection.transaction().map_err(error)?;
        if !current(&tx, token)? {
            return Ok(false);
        }
        tx.execute("UPDATE history_scans SET checked_at=?4,outcome=?5,notice=?6 WHERE agent=?1 AND invocation=?2 AND cwd=?3",
            params![agent_key(&token.source)?,token.source.invocation,token.cwd,checked_at,outcome,notice]).map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(true)
    }

    pub fn scan_summary(&self, source: &HistorySource, cwd: &str) -> Result<ScanSummary, String> {
        scope(source, cwd)?;
        let connection = self.connection.lock().map_err(error)?;
        connection.query_row("SELECT checked_at,outcome,notice FROM history_scans WHERE agent=?1 AND invocation=?2 AND cwd=?3",
            params![agent_key(source)?,source.invocation,cwd], |row| {
                let outcome = match row.get::<_,i64>(1)? {
                    0 => ScanOutcome::Never, 1 => ScanOutcome::Complete,
                    2 => ScanOutcome::Partial, 3 => ScanOutcome::Failed,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok(ScanSummary {checked_at:row.get(0)?,outcome,notice:row.get(2)?})
            }).optional().map_err(error).map(Option::unwrap_or_default)
    }

    /// Returns false when a newer scan superseded this result. Missing rows are
    /// only marked NotSeen, never treated as evidence of remote deletion.
    pub fn apply_listing(
        &self,
        token: &ScanToken,
        entries: &[ListedEntry],
        complete: bool,
        can_load: Option<bool>,
    ) -> Result<bool, String> {
        if entries.len() > MAX_ENTRIES {
            return Err(error("listing exceeds metadata capacity"));
        }
        let mut seen = HashSet::new();
        for entry in entries {
            session_id(&entry.session_id)?;
            optional(&entry.title)?;
            optional(&entry.provider_updated_at)?;
            if !seen.insert(entry.session_id.as_str()) {
                return Err(error("duplicate session ID in listing"));
            }
        }
        let mut connection = self.connection.lock().map_err(error)?;
        let tx = connection.transaction().map_err(error)?;
        if !current(&tx, token)? {
            return Ok(false);
        }
        let agent = agent_key(&token.source)?;
        let next_revision = revision(&tx).map_err(error)?;
        for entry in entries {
            tx.execute(
                "INSERT INTO history_entries (agent, invocation, cwd, session_id, title, provider_updated_at, state, can_load, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)
                 ON CONFLICT(agent, invocation, cwd, session_id) DO UPDATE SET
                 title=coalesce(excluded.title, history_entries.title),
                 provider_updated_at=coalesce(excluded.provider_updated_at, history_entries.provider_updated_at),
                 state=0, can_load=coalesce(excluded.can_load, history_entries.can_load), revision=excluded.revision
                 WHERE history_entries.revision <= ?9",
                params![agent, token.source.invocation, token.cwd, entry.session_id, entry.title, entry.provider_updated_at, can_load, next_revision, token.revision]
            ).map_err(error)?;
        }
        if complete {
            // Updated or newly created local rows (and the upserts above) have a
            // newer revision and cannot be marked absent by this older snapshot.
            tx.execute("UPDATE history_entries SET state=2, revision=?4 WHERE agent=?1 AND invocation=?2 AND cwd=?3 AND revision<=?5",
                params![agent, token.source.invocation, token.cwd, next_revision, token.revision]).map_err(error)?;
        }
        capacity(&tx, &token.source, &token.cwd)?;
        tx.commit().map_err(error)?;
        Ok(true)
    }

    /// Local lifecycle events preserve provider metadata already learned by sync.
    pub fn record_local(&self, source: &HistorySource, entry: &StoredEntry) -> Result<(), String> {
        scope(source, &entry.cwd)?;
        session_id(&entry.session_id)?;
        optional(&entry.title)?;
        optional(&entry.provider_updated_at)?;
        optional(&entry.local_activity_at)?;
        let mut connection = self.connection.lock().map_err(error)?;
        let tx = connection.transaction().map_err(error)?;
        let revision = revision(&tx).map_err(error)?;
        tx.execute("INSERT INTO history_entries VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8,?9)
            ON CONFLICT(agent,invocation,cwd,session_id) DO UPDATE SET
            title=coalesce(history_entries.title,excluded.title),
            provider_updated_at=coalesce(history_entries.provider_updated_at,excluded.provider_updated_at),
            local_activity_at=coalesce(excluded.local_activity_at,history_entries.local_activity_at),
            can_load=coalesce(excluded.can_load,history_entries.can_load),
            state=CASE WHEN history_entries.state=2 THEN 1 ELSE history_entries.state END,
            revision=excluded.revision",
            params![agent_key(source)?, source.invocation, entry.cwd, entry.session_id, entry.title, entry.provider_updated_at, entry.local_activity_at, entry.can_load, revision]).map_err(error)?;
        capacity(&tx, source, &entry.cwd)?;
        tx.commit().map_err(error)
    }

    /// Sparse ACP updates affect existing known sessions only. Explicit null and
    /// omission remain distinct, so title clearing never becomes a no-op.
    pub fn apply_info_patch(
        &self,
        source: &HistorySource,
        id: &str,
        cwd: &str,
        patch: InfoPatch,
    ) -> Result<bool, String> {
        scope(source, cwd)?;
        session_id(id)?;
        let (title_present, title) = patch_value(patch.title)?;
        let (time_present, time) = patch_value(patch.provider_updated_at)?;
        if !title_present && !time_present {
            return Ok(false);
        }
        let mut connection = self.connection.lock().map_err(error)?;
        let tx = connection.transaction().map_err(error)?;
        let revision = revision(&tx).map_err(error)?;
        let changed = tx
            .execute(
                "UPDATE history_entries SET title=CASE WHEN ?5 THEN ?6 ELSE title END,
            provider_updated_at=CASE WHEN ?7 THEN ?8 ELSE provider_updated_at END, revision=?9
            WHERE agent=?1 AND invocation=?2 AND cwd=?3 AND session_id=?4",
                params![
                    agent_key(source)?,
                    source.invocation,
                    cwd,
                    id,
                    title_present,
                    title,
                    time_present,
                    time,
                    revision
                ],
            )
            .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(changed != 0)
    }

    /// Successful replay is positive availability evidence even when list omitted
    /// the session. LocalOnly means locally observed, not exclusively Lens-created.
    pub fn mark_loaded(&self, token: &ScanToken, id: &str) -> Result<bool, String> {
        session_id(id)?;
        let mut connection = self.connection.lock().map_err(error)?;
        let tx = connection.transaction().map_err(error)?;
        if !current(&tx, token)? {
            return Ok(false);
        }
        let next_revision = revision(&tx).map_err(error)?;
        let changed = tx
            .execute(
                "UPDATE history_entries SET state=CASE WHEN state=2 THEN 1 ELSE state END,
             can_load=1, revision=?5 WHERE agent=?1 AND invocation=?2 AND cwd=?3 AND session_id=?4",
                params![
                    agent_key(&token.source)?,
                    token.source.invocation,
                    token.cwd,
                    id,
                    next_revision
                ],
            )
            .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(changed != 0)
    }

    /// Physical removal requires caller-confirmed deletion and a request token;
    /// a newer scan or row mutation makes that confirmation stale.
    #[allow(
        dead_code,
        reason = "Reserved for confirmed ACP session/delete responses; list absence is not proof"
    )]
    pub fn remove_confirmed(&self, token: &ScanToken, id: &str) -> Result<bool, String> {
        session_id(id)?;
        let mut connection = self.connection.lock().map_err(error)?;
        let tx = connection.transaction().map_err(error)?;
        if !current(&tx, token)? {
            return Ok(false);
        }
        let changed = tx.execute("DELETE FROM history_entries WHERE agent=?1 AND invocation=?2 AND cwd=?3 AND session_id=?4 AND revision<=?5",
            params![agent_key(&token.source)?, token.source.invocation, token.cwd, id, token.revision]).map_err(error)?;
        if changed != 0 {
            // Consuming the confirmation invalidates this request token too:
            // a delayed listing from that request must not recreate the row.
            let next_revision = revision(&tx).map_err(error)?;
            tx.execute(
                "UPDATE history_scans SET revision=?4 WHERE agent=?1 AND invocation=?2 AND cwd=?3",
                params![
                    agent_key(&token.source)?,
                    token.source.invocation,
                    token.cwd,
                    next_revision
                ],
            )
            .map_err(error)?;
        }
        tx.commit().map_err(error)?;
        Ok(changed != 0)
    }

    pub fn entries(&self, source: &HistorySource, cwd: &str) -> Result<Vec<StoredEntry>, String> {
        scope(source, cwd)?;
        let connection = self.connection.lock().map_err(error)?;
        let mut statement = connection.prepare("SELECT session_id,cwd,title,provider_updated_at,local_activity_at,state,can_load FROM history_entries WHERE agent=?1 AND invocation=?2 AND cwd=?3 ORDER BY session_id LIMIT ?4").map_err(error)?;
        let entries = statement
            .query_map(
                params![
                    agent_key(source)?,
                    source.invocation,
                    cwd,
                    MAX_ENTRIES as i64 + 1
                ],
                |row| {
                    let state = match row.get::<_, i64>(5)? {
                        0 => EntryState::Listed,
                        1 => EntryState::LocalOnly,
                        2 => EntryState::NotSeen,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    };
                    Ok(StoredEntry {
                        session_id: row.get(0)?,
                        cwd: row.get(1)?,
                        title: row.get(2)?,
                        provider_updated_at: row.get(3)?,
                        local_activity_at: row.get(4)?,
                        state,
                        can_load: row.get(6)?,
                    })
                },
            )
            .map_err(error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(error)?;
        if entries.len() > MAX_ENTRIES {
            return Err(error("stored metadata exceeds capacity"));
        }
        Ok(entries)
    }
}

fn patch_value(patch: Patch<String>) -> Result<(bool, Option<String>), String> {
    match patch {
        Patch::Missing => Ok((false, None)),
        Patch::Null => Ok((true, None)),
        Patch::Value(value) => {
            field(&value)?;
            Ok((true, Some(value)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> HistorySource {
        HistorySource {
            agent: AgentKind::Claude,
            invocation: "installed".into(),
        }
    }
    fn local(id: &str) -> StoredEntry {
        StoredEntry {
            session_id: id.into(),
            cwd: "/work".into(),
            title: Some("Local title".into()),
            provider_updated_at: None,
            local_activity_at: Some("2026-09-20T00:00:00Z".into()),
            state: EntryState::LocalOnly,
            can_load: None,
        }
    }
    fn listed(id: &str) -> ListedEntry {
        ListedEntry {
            session_id: id.into(),
            title: Some("Remote title".into()),
            provider_updated_at: Some("2026-09-21T00:00:00Z".into()),
        }
    }
    #[test]
    fn reopen_retains_metadata_and_rejects_unknown_schema() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        HistoryStore::open(&path)
            .unwrap()
            .record_local(&source(), &local("one"))
            .unwrap();
        let reopened = HistoryStore::open(&path).unwrap();
        assert_eq!(
            reopened.entries(&source(), "/work").unwrap(),
            vec![local("one")]
        );
        drop(reopened);
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 9)
            .unwrap();
        assert!(HistoryStore::open(&path)
            .err()
            .unwrap()
            .contains("unsupported schema"));
    }
    #[test]
    fn complete_and_partial_listings_preserve_concurrent_local_updates() {
        let store = HistoryStore::memory().unwrap();
        for id in ["old", "changed", "seen"] {
            store.record_local(&source(), &local(id)).unwrap();
        }
        let partial = store.begin_scan(&source(), "/work").unwrap();
        store
            .apply_listing(&partial, &[], false, Some(true))
            .unwrap();
        assert!(store
            .entries(&source(), "/work")
            .unwrap()
            .iter()
            .all(|e| e.state == EntryState::LocalOnly));
        let scan = store.begin_scan(&source(), "/work").unwrap();
        store
            .apply_info_patch(
                &source(),
                "changed",
                "/work",
                InfoPatch {
                    title: Patch::Value("Newer".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        store.record_local(&source(), &local("created")).unwrap();
        store
            .apply_listing(
                &scan,
                &[listed("seen"), listed("changed")],
                true,
                Some(true),
            )
            .unwrap();
        let entries = store.entries(&source(), "/work").unwrap();
        let entry = |id: &str| entries.iter().find(|e| e.session_id == id).unwrap();
        assert_eq!(entry("old").state, EntryState::NotSeen);
        assert_eq!(entry("created").state, EntryState::LocalOnly);
        assert_eq!(entry("changed").title.as_deref(), Some("Newer"));
        assert_eq!(entry("seen").state, EntryState::Listed);
        assert_eq!(entry("seen").can_load, Some(true));
    }
    #[test]
    fn newer_scan_supersedes_old_results_and_physical_deletion_is_conditional() {
        let store = HistoryStore::memory().unwrap();
        store.record_local(&source(), &local("one")).unwrap();
        let old = store.begin_scan(&source(), "/work").unwrap();
        let current = store.begin_scan(&source(), "/work").unwrap();
        assert!(!store.apply_listing(&old, &[], true, None).unwrap());
        assert!(!store.remove_confirmed(&old, "one").unwrap());
        assert!(store.remove_confirmed(&current, "one").unwrap());
        assert!(store.entries(&source(), "/work").unwrap().is_empty());
        assert!(!store
            .apply_listing(&current, &[listed("one")], true, None)
            .unwrap());
        store.record_local(&source(), &local("one")).unwrap();
        assert!(!store.remove_confirmed(&current, "one").unwrap());
        assert_eq!(store.entries(&source(), "/work").unwrap().len(), 1);
    }
    #[test]
    fn scan_status_survives_reopen_and_rejects_stale_completion() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite3");
        let store = HistoryStore::open(&path).unwrap();
        assert_eq!(
            store.scan_summary(&source(), "/work").unwrap(),
            ScanSummary::default()
        );
        let old = store.begin_scan(&source(), "/work").unwrap();
        let current = store.begin_scan(&source(), "/work").unwrap();
        assert!(!store
            .finish_scan(&old, "old", ScanOutcome::Complete, None)
            .unwrap());
        assert!(store
            .finish_scan(&current, "now", ScanOutcome::Failed, Some("unavailable"))
            .unwrap());
        drop(store);
        assert_eq!(
            HistoryStore::open(&path)
                .unwrap()
                .scan_summary(&source(), "/work")
                .unwrap(),
            ScanSummary {
                checked_at: Some("now".into()),
                outcome: ScanOutcome::Failed,
                notice: Some("unavailable".into())
            }
        );
    }

    #[test]
    fn successful_load_reconciles_absence_without_changing_activity() {
        let store = HistoryStore::memory().unwrap();
        store.record_local(&source(), &local("one")).unwrap();
        let token = store.begin_scan(&source(), "/work").unwrap();
        store.apply_listing(&token, &[], true, Some(false)).unwrap();
        assert_eq!(
            store.entries(&source(), "/work").unwrap()[0].state,
            EntryState::NotSeen
        );
        assert!(store.mark_loaded(&token, "one").unwrap());
        let row = store.entries(&source(), "/work").unwrap().remove(0);
        assert_eq!(row.state, EntryState::LocalOnly);
        assert_eq!(row.can_load, Some(true));
        assert_eq!(row.local_activity_at, local("one").local_activity_at);
        assert_eq!(row.provider_updated_at, None);
        let next = store.begin_scan(&source(), "/work").unwrap();
        assert!(!store.mark_loaded(&token, "one").unwrap());
        store
            .apply_listing(&next, &[listed("one")], true, Some(true))
            .unwrap();
        assert!(store.mark_loaded(&next, "one").unwrap());
        assert_eq!(
            store.entries(&source(), "/work").unwrap()[0].state,
            EntryState::Listed
        );
    }

    #[test]
    fn sparse_patches_and_identity_scopes_are_independent() {
        let store = HistoryStore::memory().unwrap();
        let mut changed_source = source();
        changed_source.invocation = "different".into();
        let mut changed_agent = source();
        changed_agent.agent = AgentKind::Codex;
        for source in [source(), changed_source.clone(), changed_agent.clone()] {
            store.record_local(&source, &local("one")).unwrap();
        }
        let mut elsewhere = local("one");
        elsewhere.cwd = "/elsewhere".into();
        store.record_local(&source(), &elsewhere).unwrap();
        store
            .apply_info_patch(
                &source(),
                "one",
                "/work",
                InfoPatch {
                    title: Patch::Null,
                    provider_updated_at: Patch::Value("time".into()),
                },
            )
            .unwrap();
        let row = store.entries(&source(), "/work").unwrap().remove(0);
        assert_eq!(row.title, None);
        assert_eq!(row.provider_updated_at.as_deref(), Some("time"));
        assert_eq!(row.local_activity_at, local("one").local_activity_at);
        assert!(!store
            .apply_info_patch(&source(), "one", "/work", InfoPatch::default())
            .unwrap());
        assert_eq!(
            store.entries(&changed_source, "/work").unwrap(),
            vec![local("one")]
        );
        assert_eq!(
            store.entries(&changed_agent, "/work").unwrap(),
            vec![local("one")]
        );
        assert_eq!(
            store.entries(&source(), "/elsewhere").unwrap(),
            vec![elsewhere]
        );
    }
}
