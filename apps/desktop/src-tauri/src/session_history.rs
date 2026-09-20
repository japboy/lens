//! ACP history uses its own connection and never acquires live Lens authority.
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::schema::{MaybeUndefined, ProtocolVersion};
use agent_client_protocol::{
    schema::v1::{
        ClientCapabilities, Implementation, InitializeRequest, InitializeResponse,
        ListSessionsRequest, LoadSessionRequest, SessionInfo, SessionNotification, SessionUpdate,
    },
    Agent, Client, ConnectionTo, Dispatch, DynConnectTo, Error, Handled,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::{
    model::AgentKind,
    session_document::SessionDocument,
    session_history_store::{InfoPatch, Patch},
};

const MAX_PAGES: usize = 100;
const MAX_ENTRIES: usize = 10_000;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(60);
const LOAD_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySessionEntry {
    pub session_id: String,
    pub cwd: String,
    pub title: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHistoryListing {
    pub agent: AgentKind,
    pub entries: Vec<HistorySessionEntry>,
    pub error: Option<String>,
    pub complete: bool,
    pub can_load: bool,
}

impl ProviderHistoryListing {
    fn partial(&mut self, reason: impl Into<String>) {
        self.complete = false;
        self.error = Some(reason.into());
    }
}

async fn initialize(connection: &ConnectionTo<Agent>) -> Result<InitializeResponse, Error> {
    let response = tokio::time::timeout(
        REQUEST_TIMEOUT,
        connection
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1)
                    .client_capabilities(ClientCapabilities::new())
                    .client_info(Implementation::new(
                        "lens-history",
                        env!("CARGO_PKG_VERSION"),
                    )),
            )
            .block_task(),
    )
    .await
    .map_err(|_| error("History initialization timed out"))??;
    if response.protocol_version != ProtocolVersion::V1 {
        return Err(error("Unsupported ACP history protocol version"));
    }
    Ok(response)
}

fn error(message: impl Into<String>) -> Error {
    Error::internal_error().data(message.into())
}

async fn deny_client_requests(
    dispatch: Dispatch,
    _connection: ConnectionTo<Agent>,
) -> Result<Handled<Dispatch>, Error> {
    match dispatch {
        Dispatch::Request(_, responder) => {
            responder.respond_with_error(
                Error::method_not_found()
                    .data("Client effects are unavailable during history viewing"),
            )?;
            Ok(Handled::Yes)
        }
        message => Ok(Handled::No {
            message,
            retry: false,
        }),
    }
}

/// The configuration is captured by UI admission, not recaptured after its locks are released.
fn validate_admission<R: tauri::Runtime>(
    app: &AppHandle<R>,
    agent: AgentKind,
    admitted: &crate::model::AppConfig,
) -> Result<(), String> {
    let current = app.state::<crate::app_state::AppState>().snapshot()?.config;
    if !current.same_agent_execution(admitted, agent) || !current.same_execution_config(admitted) {
        return Err("Agent configuration changed before history connection".into());
    }
    Ok(())
}

#[cfg(test)]
async fn list_transport(
    transport: DynConnectTo<Client>,
    agent: AgentKind,
    cwd: PathBuf,
) -> Result<ProviderHistoryListing, Error> {
    // Explicitly reject every client-effect request, including extension methods.
    Client
        .builder()
        .name("lens-history")
        .on_receive_dispatch(
            deny_client_requests,
            agent_client_protocol::on_receive_dispatch!(),
        )
        .connect_with(
            transport,
            move |connection: ConnectionTo<Agent>| async move {
                let initialized = initialize(&connection).await?;
                Ok(list_connection(&connection, agent, cwd, initialized).await)
            },
        )
        .await
}

async fn list_connection(
    connection: &ConnectionTo<Agent>,
    agent: AgentKind,
    cwd: PathBuf,
    initialized: InitializeResponse,
) -> ProviderHistoryListing {
    let capabilities = initialized.agent_capabilities;
    let mut listing = ProviderHistoryListing {
        agent,
        entries: Vec::new(),
        error: None,
        complete: true,
        can_load: capabilities.load_session,
    };
    if capabilities.session_capabilities.list.is_none() {
        listing.partial("Agent does not support ACP session/list");
        return listing;
    }
    let deadline = tokio::time::Instant::now() + DISCOVERY_TIMEOUT;
    let mut cursor = None;
    let mut cursors = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for page_index in 0..MAX_PAGES {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let response = tokio::time::timeout(
            remaining.min(REQUEST_TIMEOUT),
            connection
                .send_request(ListSessionsRequest::new().cwd(cwd.clone()).cursor(cursor))
                .block_task(),
        )
        .await;
        let page = match response {
            Ok(Ok(page)) => page,
            Ok(Err(cause)) => {
                listing.partial(cause.to_string());
                break;
            }
            Err(_) => {
                listing.partial("Session discovery timed out; results are incomplete");
                break;
            }
        };
        let mut exceeded_limit = false;
        for session in page.sessions {
            if session.cwd != cwd {
                continue;
            }
            if !ids.insert(session.session_id.to_string()) {
                continue;
            }
            if listing.entries.len() == MAX_ENTRIES {
                exceeded_limit = true;
                break;
            }
            listing.entries.push(entry(session));
        }
        if exceeded_limit {
            listing.partial("Session discovery entry limit reached; results are incomplete");
            break;
        }
        let Some(next) = page.next_cursor else {
            break;
        };
        if !cursors.insert(next.clone()) {
            listing.partial("Agent repeated a history cursor; results are incomplete");
            break;
        }
        if page_index + 1 == MAX_PAGES {
            listing.partial("Session discovery page limit reached; results are incomplete");
            break;
        }
        cursor = Some(next);
    }
    listing
}

fn entry(session: SessionInfo) -> HistorySessionEntry {
    HistorySessionEntry {
        session_id: session.session_id.to_string(),
        cwd: session.cwd.to_string_lossy().into_owned(),
        title: session.title,
        updated_at: session.updated_at,
    }
}

fn merge_patch(target: &mut Patch<String>, update: &MaybeUndefined<String>) {
    match update {
        MaybeUndefined::Undefined => {}
        MaybeUndefined::Null => *target = Patch::Null,
        MaybeUndefined::Value(value) => *target = Patch::Value(value.clone()),
    }
}

pub struct SyncedHistory {
    pub listing: ProviderHistoryListing,
    pub patch: InfoPatch,
    pub document: Result<SessionDocument, String>,
}

pub async fn load_synced_provider<R: tauri::Runtime>(
    app: &AppHandle<R>,
    agent: AgentKind,
    admitted: &crate::model::AppConfig,
    session_id: &str,
    cwd: &str,
) -> Result<SyncedHistory, String> {
    let cwd = PathBuf::from(cwd);
    if session_id.is_empty() || !cwd.is_absolute() {
        return Err("History requires a session ID and absolute working directory".into());
    }
    validate_admission(app, agent, admitted)?;
    let (_descriptor, transport) = crate::agent::history_transport(app, agent, cwd.clone()).await?;
    validate_admission(app, agent, admitted)?;
    load_synced_transport(transport, agent, session_id.to_owned(), cwd)
        .await
        .map_err(|e| e.to_string())
}

async fn load_synced_transport(
    transport: DynConnectTo<Client>,
    agent: AgentKind,
    session_id: String,
    cwd: PathBuf,
) -> Result<SyncedHistory, Error> {
    let collector = Arc::new(Mutex::new(ReplayCollector {
        session_id: session_id.clone(),
        collecting: false,
        document: Some(SessionDocument::default()),
        failure: None,
        patch: InfoPatch::default(),
    }));
    let notifications = Arc::clone(&collector);
    Client
        .builder()
        .name("lens-history")
        .on_receive_dispatch(
            deny_client_requests,
            agent_client_protocol::on_receive_dispatch!(),
        )
        .on_receive_notification(
            async move |notification: SessionNotification, _connection: ConnectionTo<Agent>| {
                notifications
                    .lock()
                    .map_err(|_| error("History collector lock poisoned"))?
                    .record(notification);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(
            transport,
            move |connection: ConnectionTo<Agent>| async move {
                let initialized = initialize(&connection).await?;
                let listing = list_connection(&connection, agent, cwd.clone(), initialized).await;
                let document = if listing.can_load {
                    tokio::time::timeout(
                        LOAD_TIMEOUT,
                        replay_connection(&connection, Arc::clone(&collector), session_id, cwd),
                    )
                    .await
                    .map_err(|_| "Session history loading timed out".to_string())
                    .and_then(|result| result.map_err(|e| e.to_string()))
                } else {
                    Err("Agent does not support ACP session/load".into())
                };
                let mut collected = collector
                    .lock()
                    .map_err(|_| error("History collector lock poisoned"))?;
                collected.collecting = false;
                let patch = std::mem::take(&mut collected.patch);
                Ok(SyncedHistory {
                    listing,
                    patch,
                    document,
                })
            },
        )
        .await
}

async fn replay_connection(
    connection: &ConnectionTo<Agent>,
    collector: Arc<Mutex<ReplayCollector>>,
    session_id: String,
    cwd: PathBuf,
) -> Result<SessionDocument, Error> {
    collector
        .lock()
        .map_err(|_| error("History collector lock poisoned"))?
        .collecting = true;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // The response callback freezes the replay before any later notifications dispatch.
    connection
        .send_request(LoadSessionRequest::new(session_id, cwd))
        .on_receiving_result(async move |response| {
            let mut collected = collector
                .lock()
                .map_err(|_| error("History collector lock poisoned"))?;
            collected.collecting = false;
            let result = match response {
                Ok(_) => collected.finish(),
                Err(cause) => Err(cause),
            };
            let _ = sender.send(result);
            Ok(())
        })?;
    receiver
        .await
        .map_err(|_| error("History response collector closed"))?
}

struct ReplayCollector {
    session_id: String,
    collecting: bool,
    document: Option<SessionDocument>,
    failure: Option<String>,
    patch: InfoPatch,
}

impl ReplayCollector {
    fn record(&mut self, notification: SessionNotification) {
        // Messages after the response barrier and notifications for other sessions
        // cannot mutate the document that was returned to the caller.
        if !self.collecting || notification.session_id.to_string() != self.session_id {
            return;
        }
        if let SessionUpdate::SessionInfoUpdate(ref update) = notification.update {
            merge_patch(&mut self.patch.title, &update.title);
            merge_patch(&mut self.patch.provider_updated_at, &update.updated_at);
        }
        if let Some(document) = self.document.as_mut() {
            if self.failure.is_none() {
                if let Err(cause) = document.record_update(notification.update) {
                    self.failure = Some(cause);
                }
            }
        }
    }

    fn finish(&mut self) -> Result<SessionDocument, Error> {
        self.collecting = false;
        let document = self
            .document
            .take()
            .ok_or_else(|| error("History replay already completed"))?;
        match self.failure.take() {
            Some(cause) => Err(error(cause)),
            None => Ok(document),
        }
    }
}

#[cfg(test)]
async fn load_transport(
    transport: DynConnectTo<Client>,
    session_id: String,
    cwd: PathBuf,
) -> Result<SessionDocument, Error> {
    let collector = Arc::new(Mutex::new(ReplayCollector {
        session_id: session_id.clone(),
        collecting: false,
        document: Some(SessionDocument::default()),
        failure: None,
        patch: InfoPatch::default(),
    }));
    let notifications = Arc::clone(&collector);
    Client
        .builder()
        .name("lens-history")
        .on_receive_dispatch(
            deny_client_requests,
            agent_client_protocol::on_receive_dispatch!(),
        )
        .on_receive_notification(
            async move |notification: SessionNotification, _connection: ConnectionTo<Agent>| {
                notifications
                    .lock()
                    .map_err(|_| error("History collector lock poisoned"))?
                    .record(notification);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(
            transport,
            move |connection: ConnectionTo<Agent>| async move {
                let initialized = initialize(&connection).await?;
                if !initialized.agent_capabilities.load_session {
                    return Err(error("Agent does not support ACP session/load"));
                }
                replay_connection(&connection, collector, session_id, cwd).await
            },
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::{
        schema::v1::{
            ContentBlock, ContentChunk, ListSessionsResponse, LoadSessionResponse, SessionUpdate,
            TextContent,
        },
        Responder,
    };

    fn initialized(list: bool, load: bool) -> InitializeResponse {
        serde_json::from_value(serde_json::json!({
            "protocolVersion": 1,
            "agentCapabilities": {"loadSession": load, "sessionCapabilities":
                if list { serde_json::json!({"list": {}}) } else { serde_json::json!({}) }},
            "authMethods": []
        }))
        .unwrap()
    }

    fn message(id: &str, text: &str) -> SessionNotification {
        SessionNotification::new(
            id.to_owned(),
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new(text),
            ))),
        )
    }

    #[tokio::test]
    async fn replay_freezes_at_load_response_and_ignores_other_sessions() {
        let agent = Agent
            .builder()
            .on_receive_request(
                async |_request: InitializeRequest,
                       responder: Responder<InitializeResponse>,
                       cx: ConnectionTo<Client>| {
                    cx.send_notification(message("history", "before-load"))?;
                    responder.respond(initialized(true, true))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: LoadSessionRequest,
                       responder: Responder<LoadSessionResponse>,
                       cx: ConnectionTo<Client>| {
                    assert!(request.mcp_servers.is_empty());
                    assert_eq!(request.session_id.to_string(), "history");
                    cx.send_notification(message("other", "foreign"))?;
                    cx.send_notification(message("history", "before"))?;
                    responder.respond(LoadSessionResponse::new())?;
                    cx.send_notification(message("history", "after"))?;
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            );
        let document = tokio::time::timeout(
            Duration::from_secs(5),
            load_transport(
                DynConnectTo::new(agent),
                "history".into(),
                PathBuf::from("/synthetic"),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let json = serde_json::to_value(document).unwrap();
        assert_eq!(json["entries"].as_array().unwrap().len(), 1);
        assert_eq!(json["entries"][0]["blocks"][0]["text"], "before");
    }

    #[tokio::test]
    async fn list_collects_pages_deduplicates_and_reports_cursor_cycles() {
        let agent = Agent
            .builder()
            .on_receive_request(
                async |_request: InitializeRequest,
                       responder: Responder<InitializeResponse>,
                       _cx: ConnectionTo<Client>| {
                    responder.respond(initialized(true, true))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: ListSessionsRequest,
                       responder: Responder<ListSessionsResponse>,
                       _cx: ConnectionTo<Client>| {
                    assert_eq!(request.cwd, Some(PathBuf::from("/synthetic")));
                    let mut sessions = vec![
                        SessionInfo::new("foreign", "/other"),
                        SessionInfo::new("first", "/synthetic"),
                    ];
                    if request.cursor.is_some() {
                        sessions.push(SessionInfo::new("second", "/synthetic"));
                    }
                    responder.respond(
                        ListSessionsResponse::new(sessions).next_cursor("same".to_string()),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            );
        let listing = list_transport(
            DynConnectTo::new(agent),
            AgentKind::Codex,
            PathBuf::from("/synthetic"),
        )
        .await
        .unwrap();
        assert_eq!(listing.entries.len(), 2);
        assert!(!listing.complete);
        assert!(listing.error.unwrap().contains("repeated"));
        assert!(listing.can_load);
        assert!(listing
            .entries
            .iter()
            .all(|entry| entry.updated_at.is_none()));
    }

    #[tokio::test]
    async fn failed_load_does_not_publish_partial_replay() {
        let agent = Agent
            .builder()
            .on_receive_request(
                async |_request: InitializeRequest,
                       responder: Responder<InitializeResponse>,
                       _cx: ConnectionTo<Client>| {
                    responder.respond(initialized(true, true))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_request: LoadSessionRequest,
                       responder: Responder<LoadSessionResponse>,
                       cx: ConnectionTo<Client>| {
                    cx.send_notification(message("history", "partial"))?;
                    responder.respond_with_error(error("synthetic replay failure"))
                },
                agent_client_protocol::on_receive_request!(),
            );
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            load_transport(
                DynConnectTo::new(agent),
                "history".into(),
                PathBuf::from("/synthetic"),
            ),
        )
        .await
        .unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn history_denies_agent_requested_client_file_access() {
        use agent_client_protocol::schema::v1::ReadTextFileRequest;
        let agent = Agent
            .builder()
            .on_receive_request(
                async |_request: InitializeRequest,
                       responder: Responder<InitializeResponse>,
                       _cx: ConnectionTo<Client>| {
                    responder.respond(initialized(false, true))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_request: LoadSessionRequest,
                       responder: Responder<LoadSessionResponse>,
                       cx: ConnectionTo<Client>| {
                    cx.send_request(ReadTextFileRequest::new("history", "/must-not-read"))
                        .on_receiving_result(async move |response| {
                            assert!(response.is_err());
                            responder.respond(LoadSessionResponse::new())
                        })
                },
                agent_client_protocol::on_receive_request!(),
            );
        let document = tokio::time::timeout(
            Duration::from_secs(5),
            load_synced_transport(
                DynConnectTo::new(agent),
                AgentKind::Codex,
                "history".into(),
                PathBuf::from("/synthetic"),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!document.listing.complete);
        assert!(document.document.unwrap().entries.is_empty());
    }

    #[tokio::test]
    async fn unsupported_capabilities_never_send_list_or_load() {
        let agent = Agent.builder().on_receive_request(
            async |_request: InitializeRequest,
                   responder: Responder<InitializeResponse>,
                   _cx: ConnectionTo<Client>| {
                responder.respond(initialized(false, false))
            },
            agent_client_protocol::on_receive_request!(),
        );
        let listing = list_transport(
            DynConnectTo::new(agent),
            AgentKind::Codex,
            PathBuf::from("/synthetic"),
        )
        .await
        .unwrap();
        assert!(listing.entries.is_empty());
        assert!(!listing.complete);
        assert!(!listing.can_load);
        assert!(listing.error.unwrap().contains("does not support"));
        let agent = Agent.builder().on_receive_request(
            async |_request: InitializeRequest,
                   responder: Responder<InitializeResponse>,
                   _cx: ConnectionTo<Client>| {
                responder.respond(initialized(false, false))
            },
            agent_client_protocol::on_receive_request!(),
        );
        assert!(load_transport(
            DynConnectTo::new(agent),
            "history".into(),
            PathBuf::from("/synthetic")
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("does not support"));
    }
    #[tokio::test]
    async fn synced_open_retains_listing_and_metadata_when_load_fails() {
        use agent_client_protocol::schema::v1::SessionInfoUpdate;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let initialize_calls = Arc::clone(&calls);
        let list_calls = Arc::clone(&calls);
        let load_calls = Arc::clone(&calls);
        let agent = Agent
            .builder()
            .on_receive_request(
                async move |_: InitializeRequest,
                            responder: Responder<InitializeResponse>,
                            _: ConnectionTo<Client>| {
                    initialize_calls.lock().unwrap().push("initialize");
                    responder.respond(initialized(true, true))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_: ListSessionsRequest,
                            responder: Responder<ListSessionsResponse>,
                            _: ConnectionTo<Client>| {
                    list_calls.lock().unwrap().push("list");
                    responder.respond(ListSessionsResponse::new(vec![SessionInfo::new(
                        "history",
                        "/synthetic",
                    )
                    .title("old")]))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |_: LoadSessionRequest,
                            responder: Responder<LoadSessionResponse>,
                            cx: ConnectionTo<Client>| {
                    load_calls.lock().unwrap().push("load");
                    cx.send_notification(SessionNotification::new(
                        "history",
                        SessionUpdate::SessionInfoUpdate(
                            SessionInfoUpdate::new()
                                .title("new")
                                .updated_at(MaybeUndefined::Null),
                        ),
                    ))?;
                    cx.send_notification(SessionNotification::new(
                        "history",
                        SessionUpdate::SessionInfoUpdate(
                            SessionInfoUpdate::new().title(MaybeUndefined::Null),
                        ),
                    ))?;
                    responder.respond_with_error(Error::resource_not_found(None))?;
                    cx.send_notification(SessionNotification::new(
                        "history",
                        SessionUpdate::SessionInfoUpdate(
                            SessionInfoUpdate::new().title("too late"),
                        ),
                    ))?;
                    Ok(())
                },
                agent_client_protocol::on_receive_request!(),
            );
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            load_synced_transport(
                DynConnectTo::new(agent),
                AgentKind::Codex,
                "history".into(),
                PathBuf::from("/synthetic"),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(*calls.lock().unwrap(), vec!["initialize", "list", "load"]);
        assert!(result.listing.complete);
        assert_eq!(result.listing.entries[0].title.as_deref(), Some("old"));
        assert!(result.document.is_err());
        assert!(matches!(result.patch.title, Patch::Null));
        assert!(matches!(result.patch.provider_updated_at, Patch::Null));
    }

    #[tokio::test]
    async fn synced_unsupported_preserves_incomplete_listing() {
        let agent = Agent.builder().on_receive_request(
            async |_: InitializeRequest,
                   responder: Responder<InitializeResponse>,
                   _: ConnectionTo<Client>| {
                responder.respond(initialized(false, false))
            },
            agent_client_protocol::on_receive_request!(),
        );
        let result = load_synced_transport(
            DynConnectTo::new(agent),
            AgentKind::Codex,
            "history".into(),
            PathBuf::from("/synthetic"),
        )
        .await
        .unwrap();
        assert!(!result.listing.complete);
        assert!(result.document.is_err());
    }
}
