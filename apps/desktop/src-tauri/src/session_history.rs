//! ACP history uses its own connection and never acquires live Lens authority.
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{
    schema::v1::{
        ClientCapabilities, Implementation, InitializeRequest, InitializeResponse,
        ListSessionsRequest, LoadSessionRequest, SessionInfo, SessionNotification,
    },
    Agent, Client, ConnectionTo, Dispatch, DynConnectTo, Error, Handled,
};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::{model::AgentKind, session_document::SessionDocument};

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

pub async fn list_provider<R: tauri::Runtime>(
    app: &AppHandle<R>,
    agent: AgentKind,
    cwd: &std::path::Path,
) -> Result<ProviderHistoryListing, String> {
    // Keep the installed runtime lease alive until its private transport exits.
    let (_descriptor, transport) =
        crate::agent::history_transport(app, agent, cwd.to_path_buf()).await?;
    list_transport(transport, agent, cwd.to_path_buf())
        .await
        .map_err(|e| e.to_string())
}

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
                    return Ok(listing);
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
                            .send_request(
                                ListSessionsRequest::new().cwd(cwd.clone()).cursor(cursor),
                            )
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
                        listing.partial(
                            "Session discovery entry limit reached; results are incomplete",
                        );
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
                        listing.partial(
                            "Session discovery page limit reached; results are incomplete",
                        );
                        break;
                    }
                    cursor = Some(next);
                }
                Ok(listing)
            },
        )
        .await
}

fn entry(session: SessionInfo) -> HistorySessionEntry {
    HistorySessionEntry {
        session_id: session.session_id.to_string(),
        cwd: session.cwd.to_string_lossy().into_owned(),
        title: session.title,
        updated_at: session.updated_at,
    }
}

struct ReplayCollector {
    session_id: String,
    collecting: bool,
    document: Option<SessionDocument>,
    failure: Option<String>,
}

impl ReplayCollector {
    fn record(&mut self, notification: SessionNotification) {
        // Messages after the response barrier and notifications for other sessions
        // cannot mutate the document that was returned to the caller.
        if !self.collecting || notification.session_id.to_string() != self.session_id {
            return;
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

pub async fn load_provider<R: tauri::Runtime>(
    app: &AppHandle<R>,
    agent: AgentKind,
    session_id: &str,
    cwd: &str,
) -> Result<SessionDocument, String> {
    let cwd = PathBuf::from(cwd);
    if session_id.is_empty() || !cwd.is_absolute() {
        return Err("History requires a session ID and absolute working directory".into());
    }
    let (_descriptor, transport) =
        crate::agent::history_transport(app, agent, cwd.to_path_buf()).await?;
    tokio::time::timeout(
        LOAD_TIMEOUT,
        load_transport(transport, session_id.to_owned(), cwd),
    )
    .await
    .map_err(|_| "Session history loading timed out".to_string())?
    .map_err(|e| e.to_string())
}

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
                collector
                    .lock()
                    .map_err(|_| error("History collector lock poisoned"))?
                    .collecting = true;
                let (sender, receiver) = tokio::sync::oneshot::channel();
                // Register the response callback before publication. The SDK dispatch
                // loop processes prior notifications before this callback, and holds
                // later notifications until the immutable document has been taken.
                connection
                    .send_request(LoadSessionRequest::new(session_id, cwd))
                    .on_receiving_result(async move |response| {
                        let result = match response {
                            Ok(_) => collector
                                .lock()
                                .map_err(|_| error("History collector lock poisoned"))?
                                .finish(),
                            Err(cause) => Err(cause),
                        };
                        let _ = sender.send(result);
                        Ok(())
                    })?;
                receiver
                    .await
                    .map_err(|_| error("History response collector closed"))?
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
                    responder.respond(initialized(true, true))
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
            load_transport(
                DynConnectTo::new(agent),
                "history".into(),
                PathBuf::from("/synthetic"),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(document.entries.is_empty());
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
}
