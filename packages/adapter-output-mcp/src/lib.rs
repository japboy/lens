//! Session-owned HTTP MCP publication. HTML stays inside the host process.
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Router,
};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, ServerHandler,
};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const MAX_HTML_BYTES: usize = 512 * 1024;
/// Six-byte JSON escapes plus bounded MCP protocol overhead.
pub const MAX_FRAME_BYTES: usize = MAX_HTML_BYTES * 6 + 16 * 1024;
pub type ServerError = Box<dyn std::error::Error + Send + Sync>;
/// A single `accept` failure can be transient — a client that aborts the handshake, a
/// momentary descriptor shortage — and must not retire the endpoint for the whole session.
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_CONSECUTIVE_ACCEPT_FAILURES: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedHtml {
    pub id: Uuid,
    pub html: String,
}

enum PublicationState {
    Idle,
    Active {
        turn_id: Uuid,
        publication: Option<PublishedHtml>,
    },
    Closed,
}
type Shared = Arc<Mutex<PublicationState>>;

/// Owns the listener, connections and MCP sessions. Never formats credentials.
pub struct HttpPublisher {
    endpoint_url: String,
    authorization_header: String,
    state: Shared,
    cancellation: CancellationToken,
    listener: JoinHandle<()>,
}

impl HttpPublisher {
    pub async fn start() -> Result<Self, ServerError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let authority = listener.local_addr()?.to_string();
        let origin = format!("http://{authority}");
        // Three independent UUIDv4 values provide 366 random bits.
        let authorization_header = format!(
            "Bearer {}{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        );
        let state = Arc::new(Mutex::new(PublicationState::Idle));
        let cancellation = CancellationToken::new();
        let mut config = StreamableHttpServerConfig::default();
        config.json_response = true;
        config.cancellation_token = cancellation.clone();
        config.allowed_hosts = vec![authority.clone()];
        config.allowed_origins = vec![origin.clone()];
        config.max_request_body_bytes = MAX_FRAME_BYTES;
        let service: StreamableHttpService<HtmlPublisher, LocalSessionManager> =
            StreamableHttpService::new(
                {
                    let state = state.clone();
                    move || {
                        Ok(HtmlPublisher {
                            state: state.clone(),
                        })
                    }
                },
                Default::default(),
                config,
            );
        let router =
            Router::new()
                .nest_service("/mcp", service)
                .layer(middleware::from_fn_with_state(
                    HttpBoundary {
                        authorization: authorization_header.clone(),
                        authority,
                        origin: origin.clone(),
                    },
                    guard,
                ));
        // Own every accepted connection task: aborting this task drops JoinSet,
        // aborting even idle/partial HTTP connections rather than draining forever.
        let retired = state.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            let mut consecutive_failures = 0u32;
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, _) = match accepted {
                            Ok(accepted) => {
                                consecutive_failures = 0;
                                accepted
                            }
                            // Retire only once failures persist, which means the listener
                            // itself is gone rather than a single connection failing.
                            Err(error) => {
                                consecutive_failures += 1;
                                if consecutive_failures > MAX_CONSECUTIVE_ACCEPT_FAILURES {
                                    eprintln!(
                                        "Lens output MCP listener stopped accepting connections: {error}"
                                    );
                                    // Record the terminal state the publisher already has a
                                    // variant for, so callers learn about it through the same
                                    // state machine as every other refusal.
                                    if let Ok(mut state) = retired.lock() {
                                        *state = PublicationState::Closed;
                                    }
                                    break;
                                }
                                // Awaiting the delay inside this branch parks the whole
                                // `select!`, so drain anything that already finished first
                                // rather than leaving it in the set for the retry window.
                                while connections.try_join_next().is_some() {}
                                tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                                continue;
                            }
                        };
                        let service = TowerToHyperService::new(router.clone());
                        connections.spawn(async move {
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(TokioIo::new(stream), service).await;
                        });
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            endpoint_url: format!("{origin}/mcp"),
            authorization_header,
            state,
            cancellation,
            listener: task,
        })
    }

    pub fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }
    pub fn authorization_header(&self) -> &str {
        &self.authorization_header
    }

    /// Caller supplies a fresh UUID for every generation attempt, including retries.
    pub fn begin_turn(&self, turn_id: Uuid) -> Result<TurnPublication, ServerError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("publication state unavailable"))?;
        // A retired listener is reported here rather than left for `finish` to return the
        // same `None` as an Agent that simply never published, which would leave the host
        // unable to tell a dead endpoint from an unused one.
        if matches!(*state, PublicationState::Closed) {
            return Err(io::Error::other("publication endpoint is no longer accepting").into());
        }
        if turn_id.is_nil() || !matches!(*state, PublicationState::Idle) {
            return Err(io::Error::other("publisher is not idle or turn ID is invalid").into());
        }
        *state = PublicationState::Active {
            turn_id,
            publication: None,
        };
        Ok(TurnPublication {
            state: self.state.clone(),
            turn_id,
        })
    }
}

impl Drop for HttpPublisher {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            *state = PublicationState::Closed;
        }
        self.cancellation.cancel();
        self.listener.abort();
    }
}

/// Dropping an unfinished turn discards its candidate and revokes acceptance.
pub struct TurnPublication {
    state: Shared,
    turn_id: Uuid,
}
impl TurnPublication {
    pub fn finish(self) -> Result<Option<PublishedHtml>, ServerError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("publication state unavailable"))?;
        match &mut *state {
            PublicationState::Active {
                turn_id,
                publication,
            } if *turn_id == self.turn_id => {
                let result = publication.take();
                *state = PublicationState::Idle;
                Ok(result)
            }
            _ => Err(io::Error::other("publication turn is no longer active").into()),
        }
    }
}
impl Drop for TurnPublication {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            if matches!(&*state, PublicationState::Active { turn_id, .. } if *turn_id == self.turn_id)
            {
                *state = PublicationState::Idle;
            }
        }
    }
}

#[derive(Clone)]
struct HttpBoundary {
    authorization: String,
    authority: String,
    origin: String,
}
async fn guard(
    State(boundary): State<HttpBoundary>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let headers = request.headers();
    let exact_header = |name: &str, expected: &str| {
        let mut values = headers.get_all(name).iter();
        matches!(values.next(), Some(value) if value.as_bytes() == expected.as_bytes())
            && values.next().is_none()
    };
    if !exact_header("authorization", &boundary.authorization) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if !exact_header("host", &boundary.authority) {
        return Err(StatusCode::FORBIDDEN);
    }
    if headers.contains_key("origin") && !exact_header("origin", &boundary.origin) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(next.run(request).await)
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishHtmlInput {
    /// Complete static HTML or fragment, at most 512 KiB in UTF-8.
    pub html: String,
    /// The exact turn_id provided in the current Lens prompt metadata.
    pub turn_id: String,
}
#[derive(Clone)]
struct HtmlPublisher {
    state: Shared,
}
#[tool_router]
impl HtmlPublisher {
    #[tool(
        description = "Publish a self-contained HTML artifact for display in Lens's Hero area. Supply html and the exact turn_id from the current lens_output_publication prompt metadata. Pass HTML itself, not a file path, URL, Markdown link, or fenced code block. Use expressive HTML and CSS, inline SVG and data-URL images. JavaScript, external resources and form submission are disabled. Publish at most one artifact per turn; an identical retry is accepted. HTML is registered directly with Lens without writing files or fetching network resources. Ordinary answers can remain text.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    fn publish_html(
        &self,
        Parameters(input): Parameters<PublishHtmlInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let invalid = |message| ErrorData::invalid_params(message, None);
        if input.html.trim().is_empty() || input.html.len() > MAX_HTML_BYTES {
            return Err(invalid(
                "html must be non-empty and at most 524288 UTF-8 bytes",
            ));
        }
        let requested =
            Uuid::parse_str(&input.turn_id).map_err(|_| invalid("turn_id must be a UUID"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ErrorData::internal_error("publication state unavailable", None))?;
        let PublicationState::Active {
            turn_id,
            publication,
        } = &mut *state
        else {
            return Err(invalid("turn is not active"));
        };
        if *turn_id != requested {
            return Err(invalid("turn is not active"));
        }
        let accepted = match publication {
            Some(previous) if previous.html != input.html => {
                return Err(invalid(
                    "a different HTML artifact has already been published for this turn",
                ))
            }
            Some(previous) => previous,
            None => publication.insert(PublishedHtml {
                id: Uuid::new_v4(),
                html: input.html,
            }),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::json!({"accepted": true, "publication_id": accepted.id}).to_string(),
        )]))
    }
}
#[tool_handler(name = "lens-output-mcp", version = "0.1.0")]
impl ServerHandler for HtmlPublisher {}
