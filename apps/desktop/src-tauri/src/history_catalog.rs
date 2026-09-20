//! App-owned history identity and cached projection; reading never starts an Agent.
use crate::{
    app_state::AppState,
    model::{AgentKind, AppConfig},
    session_history::ProviderHistoryListing,
    session_history_store::{HistorySource, HistoryStore, ListedEntry, ScanToken},
};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

pub(crate) fn install(state: &AppState, store: HistoryStore) -> Result<(), String> {
    let store = Arc::new(store);
    let writer = crate::history_writer::HistoryWriter::new(Arc::clone(&store))?;
    state
        .history_store
        .set(store)
        .map_err(|_| "History store already initialized")?;
    state
        .history_writer
        .set(writer)
        .map_err(|_| "History writer already initialized")?;
    Ok(())
}

pub(crate) fn writer(state: &AppState) -> Result<&crate::history_writer::HistoryWriter, String> {
    state
        .history_writer
        .get()
        .ok_or_else(|| "History persistence is unavailable".into())
}

pub(crate) fn store(state: &AppState) -> Result<Arc<HistoryStore>, String> {
    state.history_store.get().cloned().ok_or_else(|| {
        state
            .history_storage_error
            .get()
            .cloned()
            .unwrap_or_else(|| "History storage is unavailable".into())
    })
}

pub(crate) fn source(config: &AppConfig, agent: AgentKind) -> Result<HistorySource, String> {
    // Names and preferences do not identify the Agent's session namespace.
    let invocation = match agent {
        AgentKind::External(id) => {
            let profile = config
                .external_agents
                .iter()
                .find(|profile| profile.id == id)
                .ok_or("This Agent preset is no longer available")?;
            let bytes = serde_json::to_vec(&(&profile.command, &profile.args))
                .map_err(|error| error.to_string())?;
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
        AgentKind::Claude => "managed-claude-v1".into(),
        AgentKind::Codex => "managed-codex-v1".into(),
    };
    Ok(HistorySource { agent, invocation })
}

pub(crate) fn sources(config: &AppConfig) -> Result<Vec<HistorySource>, String> {
    [AgentKind::Claude, AgentKind::Codex]
        .into_iter()
        .chain(
            config
                .external_agents
                .iter()
                .map(|profile| AgentKind::External(profile.id)),
        )
        .map(|agent| source(config, agent))
        .collect()
}

pub(crate) fn same_source(config: &AppConfig, expected: &HistorySource) -> bool {
    source(config, expected.agent).is_ok_and(|current| current.invocation == expected.invocation)
}

pub(crate) fn apply_listing(
    store: &HistoryStore,
    token: &ScanToken,
    listing: &ProviderHistoryListing,
) -> Result<(), String> {
    let entries = listing
        .entries
        .iter()
        .map(|entry| ListedEntry {
            session_id: entry.session_id.clone(),
            title: entry.title.clone(),
            provider_updated_at: entry.updated_at.clone(),
        })
        .collect::<Vec<_>>();
    store
        .apply_listing(token, &entries, listing.complete, Some(listing.can_load))
        .map(|_| ())
}

pub(crate) fn now() -> String {
    chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()).to_rfc3339()
}

pub(crate) fn directory(cwd: &Path) -> Result<&str, String> {
    cwd.to_str()
        .ok_or_else(|| "History requires a UTF-8 working directory".into())
}
