//! Source-bound HTTP MCP Apps broker. One upstream session is shared by Agent and App.
use crate::ServerError;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::Mutex as AsyncMutex,
    task::{JoinHandle, JoinSet},
};
use uuid::Uuid;

pub const MAX_RESOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_RPC_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ARTIFACTS: usize = 32;
pub const FALLBACK_SERVER: &str = "lens_rich_content";
pub const FALLBACK_URI: &str = "ui://lens/rich-html.html";

#[derive(Clone)]
pub struct SourceConfig {
    pub id: String,
    pub name: String,
    pub url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppResource {
    pub html: String,
    pub meta: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppArtifact {
    pub id: Uuid,
    pub run_id: Uuid,
    pub server_id: String,
    pub tool_name: String,
    pub resource_uri: String,
    pub title: String,
    pub resource: AppResource,
    pub input: Value,
    pub result: Value,
}
struct Upstream {
    client: reqwest::Client,
    url: String,
    session_id: Option<String>,
    protocol: String,
    next_id: u64,
}
struct Source {
    id: String,
    name: String,
    peer: Value,
    tools: HashMap<String, Value>,
    upstream: Option<AsyncMutex<Upstream>>,
    shell: String,
}
struct ActiveTurn {
    id: Uuid,
    artifacts: Vec<AppArtifact>,
    retained_bytes: usize,
}
struct BrokerState {
    secret: String,
    sources: HashMap<String, Arc<Source>>,
    turn: Mutex<Option<ActiveTurn>>,
    closed: std::sync::atomic::AtomicBool,
}
pub struct AppBroker {
    state: Arc<BrokerState>,
    origin: String,
    task: JoinHandle<()>,
}
impl Drop for AppBroker {
    fn drop(&mut self) {
        self.state
            .closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.task.abort();
    }
}

impl AppBroker {
    pub async fn start(configs: Vec<SourceConfig>, shell: String) -> Result<Self, ServerError> {
        let mut sources = HashMap::new();
        for config in configs {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()?;
            let mut upstream = Upstream {
                client,
                url: config.url,
                session_id: None,
                protocol: "2025-11-25".into(),
                next_id: 0,
            };
            let peer = upstream.request("initialize", json!({"protocolVersion":"2025-11-25","capabilities":{"extensions":{"io.modelcontextprotocol/ui":{"mimeTypes":["text/html;profile=mcp-app"]}}},"clientInfo":{"name":"lens","version":env!("CARGO_PKG_VERSION")}})).await?;
            upstream.protocol = peer
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|v| matches!(*v, "2025-11-25" | "2025-06-18" | "2025-03-26"))
                .ok_or("MCP server returned an unsupported protocol version")?
                .to_owned();
            upstream.notify("notifications/initialized").await?;
            let catalog = upstream.request("tools/list", json!({})).await?;
            if catalog.get("nextCursor").is_some_and(|v| !v.is_null()) {
                return Err("Paginated MCP catalogs are not supported by this Host profile".into());
            }
            let tools = catalog
                .get("tools")
                .and_then(Value::as_array)
                .ok_or("MCP tools/list has no tools")?
                .iter()
                .map(|tool| {
                    let name = tool
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or("MCP tool has no name")?;
                    Ok((name.to_owned(), tool.clone()))
                })
                .collect::<Result<HashMap<_, _>, ServerError>>()?;
            sources.insert(
                config.id.clone(),
                Arc::new(Source {
                    id: config.id,
                    name: config.name,
                    peer,
                    tools,
                    upstream: Some(AsyncMutex::new(upstream)),
                    shell: String::new(),
                }),
            );
        }
        let tool = json!({"name":"render_html","title":"Render interactive HTML","description":"Render self-contained HTML/CSS/JavaScript as an interactive Lens interpretation. Provide complete HTML, not a path, URL or Markdown. For interactive input, send standard MCP Apps JSON-RPC to window.parent.postMessage(message, '*'). Initial HTML body LaTeX using \\(...\\) and \\[...\\] is rendered by Lens. Normal HTTP(S) anchors request a trusted Lens Open control; never open links automatically. Supported methods: ui/open-link with {url}, ui/message with {role:'user',content:[{type:'text',text:'message'}]}, ui/update-model-context with {content,structuredContent}, and tools/call with {name,arguments}. Include jsonrpc:'2.0' and a unique id. Responses arrive as window message events. User messages become a Lens draft and require the trusted Lens Send control. External network and native access are unavailable. Prefer an appropriate external renderer if one is available.","inputSchema":{"type":"object","properties":{"html":{"type":"string","maxLength":524288}},"required":["html"],"additionalProperties":false},"annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false},"_meta":{"ui":{"resourceUri":FALLBACK_URI,"visibility":["model"]}}});
        sources.insert(FALLBACK_SERVER.into(),Arc::new(Source {id:FALLBACK_SERVER.into(),name:FALLBACK_SERVER.into(),peer:json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":FALLBACK_SERVER,"version":env!("CARGO_PKG_VERSION")}}),tools:HashMap::from([("render_html".into(),tool)]),upstream:None,shell}));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(BrokerState {
            secret: format!(
                "Bearer {}{}",
                Uuid::new_v4().simple(),
                Uuid::new_v4().simple()
            ),
            sources,
            turn: Mutex::new(None),
            closed: std::sync::atomic::AtomicBool::new(false),
        });
        let router = Router::new()
            .route("/mcp/{source}", post(agent_rpc))
            .layer(axum::extract::DefaultBodyLimit::max(MAX_RPC_BYTES))
            .with_state(Arc::clone(&state));
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {accepted=listener.accept()=>{let Ok((stream,_))=accepted else {break};let service=TowerToHyperService::new(router.clone());connections.spawn(async move {let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).await;});},Some(_)=connections.join_next(),if !connections.is_empty()=>{}}
            }
        });
        Ok(Self {
            state,
            origin,
            task,
        })
    }
    pub fn registrations(&self) -> Vec<(String, String, String)> {
        let mut registrations: Vec<_> = self
            .state
            .sources
            .values()
            .map(|s| {
                (
                    s.name.clone(),
                    format!("{}/mcp/{}", self.origin, s.id),
                    self.state.secret.clone(),
                )
            })
            .collect();
        registrations.sort_by(|a, b| a.0.cmp(&b.0));
        registrations
    }

    /// Close owned routing first, then best-effort release upstream HTTP sessions.
    pub async fn close(&self) {
        self.state
            .closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.task.abort();
        if let Ok(mut turn) = self.state.turn.lock() {
            *turn = None;
        }
        let cleanup = async {
            for source in self.state.sources.values() {
                if let Some(upstream) = &source.upstream {
                    let cleanup = async {
                        let upstream = upstream.lock().await;
                        if let Some(id) = &upstream.session_id {
                            let _ = upstream
                                .client
                                .delete(&upstream.url)
                                .header("MCP-Session-Id", id)
                                .header("MCP-Protocol-Version", &upstream.protocol)
                                .send()
                                .await;
                        }
                    };
                    let _ = tokio::time::timeout(Duration::from_secs(2), cleanup).await;
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(3), cleanup).await;
    }
    pub fn begin_turn(&self, id: Uuid) -> Result<(), String> {
        if id.is_nil() {
            return Err("MCP turn identity must be non-nil".into());
        }
        let mut turn = self
            .state
            .turn
            .lock()
            .map_err(|_| "MCP turn state unavailable")?;
        if turn.is_some() {
            return Err("MCP turn is busy".into());
        }
        *turn = Some(ActiveTurn {
            id,
            artifacts: Vec::new(),
            retained_bytes: 0,
        });
        Ok(())
    }
    pub fn finish_turn(&self, id: Uuid) -> Result<Vec<AppArtifact>, String> {
        let mut turn = self
            .state
            .turn
            .lock()
            .map_err(|_| "MCP turn state unavailable")?;
        if turn.as_ref().is_none_or(|t| t.id != id) {
            return Err("MCP turn authority expired".into());
        }
        Ok(turn.take().expect("checked turn").artifacts)
    }
    pub fn abort_turn(&self, id: Uuid) {
        if let Ok(mut turn) = self.state.turn.lock() {
            if turn.as_ref().is_some_and(|t| t.id == id) {
                *turn = None;
            }
        }
    }
    pub async fn call_from_app(
        &self,
        artifact: &AppArtifact,
        params: Value,
    ) -> Result<Value, String> {
        if self.state.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err("MCP source connection closed".into());
        }
        let source = self
            .state
            .sources
            .get(&artifact.server_id)
            .ok_or("MCP source unavailable")?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or("Missing App tool name")?;
        if let Some(upstream) = &source.upstream {
            let mut upstream = tokio::time::timeout(Duration::from_secs(30), upstream.lock())
                .await
                .map_err(|_| "MCP source is busy")?;
            let catalog = upstream
                .request("tools/list", json!({}))
                .await
                .map_err(|_| "Unable to refresh App tool authority")?;
            if catalog.get("nextCursor").is_some_and(|v| !v.is_null()) {
                return Err("Paginated catalogs are unsupported".into());
            }
            let tool = catalog
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|t| {
                    t.iter()
                        .find(|t| t.get("name").and_then(Value::as_str) == Some(name))
                })
                .ok_or("App tool is no longer advertised")?;
            if !visible(tool, "app") {
                return Err("Tool is not App-visible".into());
            }
            upstream
                .request("tools/call", params)
                .await
                .map_err(|_| "MCP App tool call failed".into())
        } else {
            let tool = source.tools.get(name).ok_or("Unknown App tool")?;
            if !visible(tool, "app") {
                return Err("Tool is not App-visible".into());
            }
            source
                .call("tools/call", params)
                .await
                .map_err(|_| "MCP App tool call failed".into())
        }
    }
    pub async fn list_from_app(&self, artifact: &AppArtifact) -> Result<Value, String> {
        if self.state.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err("MCP source connection closed".into());
        }
        let source = self
            .state
            .sources
            .get(&artifact.server_id)
            .ok_or("MCP source unavailable")?;
        let catalog = if source.upstream.is_some() {
            source
                .call("tools/list", json!({}))
                .await
                .map_err(|_| "Unable to refresh App tool authority")?
        } else {
            json!({"tools":source.tools.values().collect::<Vec<_>>()})
        };
        if catalog.get("nextCursor").is_some_and(|v| !v.is_null()) {
            return Err("Paginated catalogs are unsupported".into());
        }
        let mut tools: Vec<_> = catalog
            .get("tools")
            .and_then(Value::as_array)
            .ok_or("Invalid App tool catalog")?
            .iter()
            .filter(|tool| visible(tool, "app"))
            .cloned()
            .collect();
        tools.sort_by(|a, b| {
            a.get("name")
                .and_then(Value::as_str)
                .cmp(&b.get("name").and_then(Value::as_str))
        });
        Ok(json!({"tools":tools}))
    }
}
impl Upstream {
    async fn notify(&mut self, method: &str) -> Result<(), ServerError> {
        self.send(json!({"jsonrpc":"2.0","method":method})).await?;
        Ok(())
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, ServerError> {
        self.next_id += 1;
        let response = self
            .send(json!({"jsonrpc":"2.0","id":self.next_id,"method":method,"params":params}))
            .await?
            .ok_or("MCP server returned no response")?;
        if response.get("error").is_some() {
            return Err("MCP request was rejected".into());
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| "MCP response has no result".into())
    }
    async fn send(&mut self, message: Value) -> Result<Option<Value>, ServerError> {
        let mut request = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", &self.protocol)
            .json(&message);
        if let Some(id) = &self.session_id {
            request = request.header("MCP-Session-Id", id);
        }
        let mut response = request.send().await?.error_for_status()?;
        if let Some(id) = response.headers().get("mcp-session-id") {
            self.session_id = Some(id.to_str()?.to_owned());
        }
        if response.status() == reqwest::StatusCode::ACCEPTED {
            return Ok(None);
        }
        let sse = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() + chunk.len() > MAX_RPC_BYTES {
                return Err("MCP response exceeds Host size limit".into());
            }
            body.extend_from_slice(&chunk);
            if sse {
                if let Some(value) = sse_result(&body, message.get("id"))? {
                    return Ok(Some(value));
                }
            }
        }
        if body.is_empty() {
            return Ok(None);
        }
        if sse {
            return Err("MCP SSE response ended without the requested response".into());
        }
        let value: Value = serde_json::from_slice(&body)?;
        if value.get("id") != message.get("id") {
            return Err("MCP response identity mismatch".into());
        }
        Ok(Some(value))
    }
}
fn sse_result(bytes: &[u8], id: Option<&Value>) -> Result<Option<Value>, ServerError> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(None);
    };
    for event in text
        .replace("\r\n", "\n")
        .split_inclusive("\n\n")
        .filter(|e| e.ends_with("\n\n"))
    {
        let data = event
            .lines()
            .filter_map(|l| l.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        if value.get("id") == id && (value.get("result").is_some() || value.get("error").is_some())
        {
            return Ok(Some(value));
        }
    }
    Ok(None)
}
fn visible(tool: &Value, audience: &str) -> bool {
    match tool.pointer("/_meta/ui/visibility") {
        None => true,
        Some(Value::Array(values)) => values.iter().any(|v| v.as_str() == Some(audience)),
        Some(_) => false,
    }
}
impl Source {
    async fn call(&self, method: &str, params: Value) -> Result<Value, ServerError> {
        if let Some(upstream) = &self.upstream {
            return tokio::time::timeout(Duration::from_secs(30), async {
                upstream.lock().await.request(method, params).await
            })
            .await
            .map_err(|_| "MCP source request timed out")?;
        }
        match method {
            "tools/call" => Err("Bundled HTML publication requires an active Agent turn".into()),
            "resources/read" => {
                if params.get("uri").and_then(Value::as_str) != Some(FALLBACK_URI) {
                    return Err("Unknown UI resource".into());
                }
                Ok(
                    json!({"contents":[{"uri":FALLBACK_URI,"mimeType":"text/html;profile=mcp-app","text":self.shell}]}),
                )
            }
            _ => Err("Unsupported built-in request".into()),
        }
    }
}
async fn agent_rpc(
    State(state): State<Arc<BrokerState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if headers.get_all("authorization").iter().count() != 1
        || headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(state.secret.as_str())
        || headers.contains_key("origin")
    {
        return (StatusCode::FORBIDDEN, Json(Value::Null));
    }
    let message = match serde_json::from_slice::<Value>(&body) {
        Ok(value) => value,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(Value::Null)),
    };
    let request_id = message.get("id").cloned();
    let Some(source) = state.sources.get(&id) else {
        return (StatusCode::NOT_FOUND, Json(Value::Null));
    };
    let response = proxy_request(&state, source, &message).await;
    if request_id.is_none() {
        return (StatusCode::ACCEPTED, Json(Value::Null));
    }
    let envelope = match response {
        Ok(result) => json!({"jsonrpc":"2.0","id":request_id,"result":result}),
        Err(_) => {
            json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32000,"message":"Lens MCP source request failed or is unsupported"}})
        }
    };
    (StatusCode::OK, Json(envelope))
}
async fn read_app_resource(source: &Source, uri: &str) -> Result<AppResource, ServerError> {
    let resources = source.call("resources/read", json!({"uri":uri})).await?;
    let contents = resources
        .get("contents")
        .and_then(Value::as_array)
        .ok_or("UI resource has no contents")?;
    let resource = contents
        .iter()
        .find(|r| {
            r.get("uri").and_then(Value::as_str) == Some(uri)
                && r.get("mimeType").and_then(Value::as_str) == Some("text/html;profile=mcp-app")
        })
        .ok_or("UI resource is not an App")?;
    let html = resource
        .get("text")
        .and_then(Value::as_str)
        .filter(|h| !h.is_empty() && h.len() <= MAX_RESOURCE_BYTES)
        .ok_or("UI resource exceeds size limit")?;
    Ok(AppResource {
        html: html.into(),
        meta: resource.get("_meta").cloned().unwrap_or(json!({})),
    })
}

async fn proxy_request(
    state: &BrokerState,
    source: &Source,
    message: &Value,
) -> Result<Value, ServerError> {
    if state.closed.load(std::sync::atomic::Ordering::Acquire) {
        return Err("Source expired".into());
    }
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .ok_or("Invalid MCP method")?;
    let params = message.get("params").cloned().unwrap_or(json!({}));
    match method {
        "initialize" => {
            let mut peer = source.peer.clone();
            if let Some(version) = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|v| matches!(*v, "2025-11-25" | "2025-06-18" | "2025-03-26"))
            {
                peer["protocolVersion"] = json!(version);
            }
            peer["capabilities"] = json!({"tools":{},"resources":{}});
            Ok(peer)
        }
        "notifications/initialized" | "notifications/cancelled" | "ping" => Ok(json!({})),
        "tools/list" => {
            let mut tools: Vec<_> = source
                .tools
                .values()
                .filter(|t| visible(t, "model"))
                .collect();
            tools.sort_by(|a, b| {
                a.get("name")
                    .and_then(Value::as_str)
                    .cmp(&b.get("name").and_then(Value::as_str))
            });
            Ok(json!({"tools":tools}))
        }
        "resources/list" => Ok(json!({"resources":[]})),
        "tools/call" => {
            let run_id = state
                .turn
                .lock()
                .map_err(|_| "MCP turn unavailable")?
                .as_ref()
                .map(|t| t.id)
                .ok_or("No active Agent turn")?;
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or("Missing tool name")?;
            let tool = source
                .tools
                .get(name)
                .filter(|t| visible(t, "model"))
                .ok_or("Tool not model-visible")?;
            if source.id == FALLBACK_SERVER {
                return publish_builtin(state, source, &params, run_id);
            }
            let result = source.call(method, params.clone()).await?;
            let uri = tool
                .pointer("/_meta/ui/resourceUri")
                .or_else(|| tool.pointer("/_meta/ui~1resourceUri"))
                .and_then(Value::as_str);
            if let Some(uri) = uri.filter(|uri| uri.starts_with("ui://")) {
                let resource =
                    read_app_resource(source, uri)
                        .await
                        .unwrap_or_else(|_| AppResource {
                            html: String::new(),
                            meta: json!({"lens/resourceUnavailable":true}),
                        });
                let artifact = AppArtifact {
                    id: Uuid::new_v4(),
                    run_id,
                    server_id: source.id.clone(),
                    tool_name: name.to_owned(),
                    resource_uri: uri.to_owned(),
                    title: tool
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or(name)
                        .to_owned(),
                    resource,
                    input: params.get("arguments").cloned().unwrap_or(json!({})),
                    result: result.clone(),
                };
                let bytes = serde_json::to_vec(&artifact)?.len();
                let mut active = state.turn.lock().map_err(|_| "MCP turn unavailable")?;
                let Some(active) = active.as_mut().filter(|t| t.id == run_id) else {
                    return Ok(result);
                };
                if active.artifacts.len() >= MAX_ARTIFACTS
                    || active.retained_bytes + bytes > 16 * 1024 * 1024
                {
                    // A successful upstream side effect must not become a retriable MCP
                    // error merely because this Host cannot retain another presentation.
                    return Ok(result);
                }
                active.retained_bytes += bytes;
                active.artifacts.push(artifact);
            }
            Ok(result)
        }
        // Model resource access uses the same source connection; unknown methods are explicit.
        "resources/read" => source.call(method, params).await,
        _ => Err("Unsupported MCP method".into()),
    }
}

fn publish_builtin(
    state: &BrokerState,
    source: &Source,
    params: &Value,
    run_id: Uuid,
) -> Result<Value, ServerError> {
    let args = params
        .get("arguments")
        .and_then(Value::as_object)
        .ok_or("Missing render arguments")?;
    if args.len() != 1 {
        return Err("Only html is accepted".into());
    }
    let html = args
        .get("html")
        .and_then(Value::as_str)
        .filter(|html| !html.trim().is_empty() && html.len() <= crate::MAX_HTML_BYTES)
        .ok_or("Invalid HTML size")?;
    let mut turn = state.turn.lock().map_err(|_| "MCP turn unavailable")?;
    let turn = turn
        .as_mut()
        .filter(|turn| turn.id == run_id)
        .ok_or("Agent turn expired")?;
    if let Some(previous) = turn
        .artifacts
        .iter()
        .find(|artifact| artifact.server_id == FALLBACK_SERVER && artifact.input == json!(args))
    {
        return Ok(previous.result.clone());
    }
    let id = Uuid::new_v4();
    let digest: String = Sha256::digest(html.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let receipt = json!({"kind":"lens_rich_html_publication","schema_version":1,"accepted":true,"publication_id":id,"turn_id":run_id,"html_sha256":digest});
    let result = json!({"content":[{"type":"text","text":receipt.to_string()}],"structuredContent":{"html":html,"publication":receipt}});
    let artifact = AppArtifact {
        id,
        run_id,
        server_id: FALLBACK_SERVER.into(),
        tool_name: "render_html".into(),
        resource_uri: FALLBACK_URI.into(),
        title: "Render interactive HTML".into(),
        resource: AppResource {
            html: source.shell.clone(),
            meta: json!({}),
        },
        input: json!(args),
        result: result.clone(),
    };
    let bytes = serde_json::to_vec(&artifact)?.len();
    if turn.artifacts.len() >= MAX_ARTIFACTS || turn.retained_bytes + bytes > 16 * 1024 * 1024 {
        return Err("App publication budget is full".into());
    }
    turn.retained_bytes += bytes;
    turn.artifacts.push(artifact);
    Ok(result)
}

/// Validate server-declared CSP origins without interpolating arbitrary directive text.
pub fn resource_csp(resource: &AppResource) -> Result<String, String> {
    resource_policy(resource).map(|(header, _)| header)
}

/// The SDK advertisement and HTTP header derive from the same validated origins.
pub fn resource_sandbox(resource: &AppResource) -> Result<Value, String> {
    resource_policy(resource).map(|(_, sandbox)| sandbox)
}

fn resource_policy(resource: &AppResource) -> Result<(String, Value), String> {
    let ui = resource.meta.get("ui").unwrap_or(&resource.meta);
    if !ui.is_object()
        || ui.get("domain").is_some()
        || ui
            .get("permissions")
            .is_some_and(|p| p.as_object().is_none_or(|p| !p.is_empty()))
    {
        return Err("Dedicated origins and device permissions are unsupported".into());
    }
    let csp = ui.get("csp");
    if csp.is_some_and(|c| !c.is_object()) {
        return Err("Invalid App CSP metadata".into());
    }
    let sources = |field: &str, connect: bool| -> Result<Vec<String>, String> {
        let Some(value) = csp.and_then(|c| c.get(field)) else {
            return Ok(Vec::new());
        };
        let values = value
            .as_array()
            .filter(|v| v.len() <= 32)
            .ok_or("Invalid or oversized App CSP source list")?;
        values
            .iter()
            .map(|v| {
                let value = v
                    .as_str()
                    .filter(|v| v.len() <= 256)
                    .ok_or("Invalid CSP origin")?;
                let url = reqwest::Url::parse(value)
                    .map_err(|_| "CSP sources must be explicit HTTP(S) origins")?;
                if !(matches!(url.scheme(), "http" | "https")
                    || connect && matches!(url.scheme(), "ws" | "wss"))
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || url.path() != "/"
                {
                    return Err("Unsupported CSP source syntax".into());
                }
                let host = url.host_str().ok_or("CSP source has no host")?;
                let host = host.strip_prefix("*.").unwrap_or(host);
                if host.is_empty()
                    || !host.bytes().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b':' | b'[' | b']')
                    })
                {
                    return Err("Invalid CSP source host".into());
                }
                Ok(url.as_str().trim_end_matches('/').to_owned())
            })
            .collect()
    };
    let resource_domains = sources("resourceDomains", false)?;
    let connect_domains = sources("connectDomains", true)?;
    let resources = resource_domains.join(" ");
    let connects = connect_domains.join(" ");
    if ["frameDomains", "baseUriDomains"].iter().any(|field| {
        csp.and_then(|c| c.get(field))
            .is_some_and(|v| v.as_array().is_none_or(|v| !v.is_empty()))
    }) {
        return Err("External frames and base origins are unsupported".into());
    }
    Ok((format!("default-src 'none'; script-src 'unsafe-inline' {resources}; style-src 'unsafe-inline' {resources}; img-src data: blob: {resources}; font-src data: {resources}; media-src data: blob: {resources}; connect-src {}; frame-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'",if connects.is_empty(){"'none'"}else{&connects}), json!({"csp":{"resourceDomains":resource_domains,"connectDomains":connect_domains},"permissions":{}})))
}

/// Independent origin for trusted Sandbox proxy and opaque Views. No native authority.
pub struct DisplayServer {
    pub origin: String,
    pub proxy_url: String,
    pub view_url: String,
    pub document_url: Option<String>,
    document: Arc<Mutex<Option<String>>>,
    task: JoinHandle<()>,
}
impl Drop for DisplayServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl DisplayServer {
    /// Presentation derivative only; the original resource and tool transaction stay immutable.
    pub fn prepare_document(&self, document: String) -> Result<(), String> {
        if document.len() > MAX_RPC_BYTES {
            return Err("HTML presentation exceeds size limit".into());
        }
        let mut current = self
            .document
            .lock()
            .map_err(|_| "HTML presentation unavailable")?;
        if current.is_none() {
            return Err("Only bundled HTML documents can be prepared".into());
        }
        *current = Some(document);
        Ok(())
    }
    pub async fn start(
        resource: AppResource,
        document: Option<String>,
        host_origin: String,
        proxy_html: String,
        proxy_js: String,
    ) -> Result<Self, ServerError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let token = Uuid::new_v4().simple().to_string();
        let proxy_url = format!("{origin}/sandbox/{token}");
        let view_url = format!("{origin}/view/{token}");
        let document_url = document
            .as_ref()
            .map(|_| format!("{origin}/document/{token}"));
        let settings = json!({"hostOrigin":host_origin,"viewUrl":view_url})
            .to_string()
            .replace('<', "\\u003c");
        let proxy_html = proxy_html.replace("__SETTINGS_JSON__", &settings);
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let route_ready = Arc::clone(&ready);
        let html = resource.html.clone();
        let route_html = html.clone();
        let resource_policy = resource_csp(&resource).map_err(std::io::Error::other)?;
        let csp="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'";
        let router=Router::new()
            .route(&format!("/sandbox/{token}"),axum::routing::get(move||{let html=proxy_html.clone();async move {([(axum::http::header::CONTENT_TYPE,"text/html; charset=utf-8"),(axum::http::header::CONTENT_SECURITY_POLICY,"default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self'; frame-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'")],html)}}))
            .route("/sandbox-proxy.js",axum::routing::get(move||{let js=proxy_js.clone();async move {([(axum::http::header::CONTENT_TYPE,"text/javascript; charset=utf-8")],js)}}))
            .route(&format!("/view/{token}"),post(move|body:Bytes|{let html=route_html.clone();let ready=Arc::clone(&route_ready);async move {if body.as_ref()!=html.as_bytes(){return StatusCode::FORBIDDEN;}ready.store(true,std::sync::atomic::Ordering::Release);StatusCode::OK}}).get(move||{let html=html.clone();let ready=Arc::clone(&ready);let resource_policy=resource_policy.clone();async move {if !ready.load(std::sync::atomic::Ordering::Acquire){return(StatusCode::NOT_FOUND,[(axum::http::header::CONTENT_TYPE,"text/html".to_owned()),(axum::http::header::CONTENT_SECURITY_POLICY,resource_policy)],String::new());}(StatusCode::OK,[(axum::http::header::CONTENT_TYPE,"text/html; charset=utf-8".to_owned()),(axum::http::header::CONTENT_SECURITY_POLICY,resource_policy)],html)}}))
            .layer(axum::extract::DefaultBodyLimit::max(MAX_RPC_BYTES));
        let document = Arc::new(Mutex::new(document));
        let route_document = Arc::clone(&document);
        let router = if document_url.is_some() {
            router.route(
                &format!("/document/{token}"),
                axum::routing::get(move || {
                    let html = route_document
                        .lock()
                        .ok()
                        .and_then(|document| document.clone())
                        .unwrap_or_default();
                    async move {
                        (
                            [
                                (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
                                (axum::http::header::CONTENT_SECURITY_POLICY, csp),
                            ],
                            html,
                        )
                    }
                }),
            )
        } else {
            router
        };
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {accepted=listener.accept()=>{let Ok((stream,_))=accepted else{break};let service=TowerToHyperService::new(router.clone());connections.spawn(async move{let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).await;});},Some(_)=connections.join_next(),if !connections.is_empty()=>{}}
            }
        });
        Ok(Self {
            origin,
            proxy_url,
            view_url,
            document_url,
            document,
            task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_waits_for_complete_delimiter_and_matching_response() {
        let event = b"data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"text\":\"ok\"}}";
        assert!(sse_result(event, Some(&json!(7))).unwrap().is_none());
        let mut complete = event.to_vec();
        complete.extend_from_slice(b"\r\n\r\n");
        assert_eq!(
            sse_result(&complete, Some(&json!(7))).unwrap().unwrap()["result"]["text"],
            "ok"
        );
        assert!(sse_result(&complete, Some(&json!(8))).unwrap().is_none());
        let utf8 = "data: {\"id\":7,\"result\":\"\u{65e5}\u{672c}\u{8a9e}\"}\n\n".as_bytes();
        let split = utf8.iter().position(|b| *b >= 128).unwrap() + 1;
        assert!(sse_result(&utf8[..split], Some(&json!(7)))
            .unwrap()
            .is_none());
        assert_eq!(
            sse_result(utf8, Some(&json!(7))).unwrap().unwrap()["result"],
            "\u{65e5}\u{672c}\u{8a9e}"
        );
    }

    #[test]
    fn declared_child_csp_accepts_only_bounded_origins() {
        let resource = |csp| AppResource {
            html: "<p>app</p>".into(),
            meta: json!({"ui":{"csp":csp}}),
        };
        let policy = resource_csp(&resource(json!({"resourceDomains":["https://cdn.example"],"connectDomains":["wss://data.example:443"]}))).unwrap();
        assert!(policy.contains("script-src 'unsafe-inline' https://cdn.example"));
        assert!(policy.contains("connect-src wss://data.example"));
        assert!(!policy.contains("unsafe-eval"));
        for invalid in [
            json!({"resourceDomains":["https://cdn.example/path"]}),
            json!({"connectDomains":["https://user:secret@data.example"]}),
            json!({"resourceDomains":["https://cdn.example; script-src *"]}),
            json!({"frameDomains":["https://frames.example"]}),
            json!({"frameDomains":"bad"}),
        ] {
            assert!(resource_csp(&resource(invalid)).is_err());
        }
    }

    #[tokio::test]
    async fn display_retains_exact_html_and_releases_owned_listener() {
        let html =
            "<!doctype html><script>window.onload=()=>{};</script><p>\u{65e5}\u{672c}\u{8a9e}</p>";
        let display = DisplayServer::start(
            AppResource {
                html: html.into(),
                meta: json!({}),
            },
            Some(html.into()),
            "tauri://localhost".into(),
            "<script src='/sandbox-proxy.js'></script>__SETTINGS_JSON__".into(),
            "/* proxy */".into(),
        )
        .await
        .unwrap();
        let client = reqwest::Client::new();
        assert_eq!(
            client.get(&display.view_url).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            client
                .post(&display.view_url)
                .body("different")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .post(&display.view_url)
                .header("content-type", "text/html")
                .body(html)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let response = client.get(&display.view_url).send().await.unwrap();
        assert_eq!(response.text().await.unwrap(), html);
        let response = client
            .get(display.document_url.as_ref().unwrap())
            .send()
            .await
            .unwrap();
        assert!(response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("connect-src 'none'"));
        assert_eq!(response.text().await.unwrap(), html);
        let endpoint = display.proxy_url.clone();
        drop(display);
        tokio::task::yield_now().await;
        assert!(client.get(endpoint).send().await.is_err());
    }

    #[tokio::test]
    async fn builtin_inventory_replaces_old_publisher_and_receipts_are_run_scoped() {
        let broker = AppBroker::start(Vec::new(), "<p>shell</p>".into())
            .await
            .unwrap();
        let source = broker.state.sources.get(FALLBACK_SERVER).unwrap();
        assert_eq!(
            broker
                .registrations()
                .iter()
                .map(|entry| entry.0.as_str())
                .collect::<Vec<_>>(),
            vec![FALLBACK_SERVER]
        );
        let list = proxy_request(&broker.state, source, &json!({"method":"tools/list"}))
            .await
            .unwrap();
        assert_eq!(list["tools"].as_array().unwrap().len(), 1);
        assert_eq!(list["tools"][0]["name"], "render_html");
        let resource = proxy_request(
            &broker.state,
            source,
            &json!({"method":"resources/read","params":{"uri":FALLBACK_URI}}),
        )
        .await
        .unwrap();
        assert_eq!(resource["contents"][0]["uri"], FALLBACK_URI);
        assert_eq!(
            resource["contents"][0]["mimeType"],
            "text/html;profile=mcp-app"
        );
        assert_eq!(resource["contents"][0]["text"], "<p>shell</p>");
        assert!(proxy_request(
            &broker.state,
            source,
            &json!({"method":"resources/read","params":{"uri":"ui://unknown/resource"}})
        )
        .await
        .is_err());

        let request = |html: Value| json!({"method":"tools/call","params":{"name":"render_html","arguments":{"html":html}}});
        assert!(
            proxy_request(&broker.state, source, &request(json!("<p>A</p>")))
                .await
                .is_err()
        );
        assert!(broker.begin_turn(Uuid::nil()).is_err());
        let run = Uuid::new_v4();
        broker.begin_turn(run).unwrap();
        let first = proxy_request(&broker.state, source, &request(json!("<p>A</p>")))
            .await
            .unwrap();
        let retry = proxy_request(&broker.state, source, &request(json!("<p>A</p>")))
            .await
            .unwrap();
        assert_eq!(first, retry);
        let other = proxy_request(&broker.state, source, &request(json!("<p>B</p>")))
            .await
            .unwrap();
        assert_ne!(
            first["structuredContent"]["publication"]["publication_id"],
            other["structuredContent"]["publication"]["publication_id"]
        );
        let receipt = &first["structuredContent"]["publication"];
        assert_eq!(receipt["turn_id"], json!(run));
        assert_eq!(receipt["schema_version"], 1);
        assert_eq!(receipt["kind"], "lens_rich_html_publication");
        let digest: String = Sha256::digest(b"<p>A</p>")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(receipt["html_sha256"], digest);
        assert!(proxy_request(
            &broker.state,
            source,
            &request(json!("x".repeat(crate::MAX_HTML_BYTES + 1)))
        )
        .await
        .is_err());
        assert!(proxy_request(&broker.state,source,&json!({"method":"tools/call","params":{"name":"publish_html","arguments":{"html":"<p>A</p>"}}})).await.is_err());
        let artifacts = broker.finish_turn(run).unwrap();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(json!(artifacts[0].id), receipt["publication_id"]);
        let next = Uuid::new_v4();
        broker.begin_turn(next).unwrap();
        let next_result = proxy_request(&broker.state, source, &request(json!("<p>A</p>")))
            .await
            .unwrap();
        assert_ne!(
            next_result["structuredContent"]["publication"]["publication_id"],
            receipt["publication_id"]
        );
        broker.close().await;
    }
    #[tokio::test]
    async fn prepared_document_is_separate_from_original_resource_and_bounded() {
        let display = DisplayServer::start(
            AppResource {
                html: "<p>original shell</p>".into(),
                meta: json!({}),
            },
            Some("<p>original document</p>".into()),
            "tauri://localhost".into(),
            "__SETTINGS_JSON__".into(),
            "".into(),
        )
        .await
        .unwrap();
        assert!(display
            .prepare_document("x".repeat(MAX_RPC_BYTES + 1))
            .is_err());
        display
            .prepare_document("<p>presentation derivative</p>".into())
            .unwrap();
        let client = reqwest::Client::new();
        assert_eq!(
            client
                .get(display.document_url.as_ref().unwrap())
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "<p>presentation derivative</p>"
        );
        assert_eq!(
            client
                .post(&display.view_url)
                .body("<p>original shell</p>")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .get(&display.view_url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "<p>original shell</p>"
        );
        let external = DisplayServer::start(
            AppResource {
                html: "<p>external</p>".into(),
                meta: json!({}),
            },
            None,
            "tauri://localhost".into(),
            "".into(),
            "".into(),
        )
        .await
        .unwrap();
        assert!(external.prepare_document("<p>changed</p>".into()).is_err());
    }
    #[tokio::test]
    async fn builtin_broker_preserves_raw_transaction_and_does_not_authorize_model_only_tools() {
        let broker = AppBroker::start(Vec::new(), "<p>fixed App</p>".into())
            .await
            .unwrap();
        let run = Uuid::new_v4();
        broker.begin_turn(run).unwrap();
        let (name, url, secret) = broker.registrations().pop().unwrap();
        assert_eq!(name, FALLBACK_SERVER);
        let original = json!({"html":"<!doctype html><p>unchanged</p>"});
        let response: Value = reqwest::Client::new().post(url).header("authorization",secret).json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"render_html","arguments":original}})).send().await.unwrap().json().await.unwrap();
        let artifacts = broker.finish_turn(run).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].input, original);
        assert_eq!(artifacts[0].result, response["result"]);
        assert_eq!(artifacts[0].resource.html, "<p>fixed App</p>");
        assert_eq!(
            broker.list_from_app(&artifacts[0]).await.unwrap(),
            json!({"tools":[]})
        );
        assert!(broker
            .call_from_app(
                &artifacts[0],
                json!({"name":"render_html","arguments":original})
            )
            .await
            .is_err());
        broker.close().await;
    }
    async fn fixture_source(
        tag: &'static str,
        fail_resource: bool,
        gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    ) -> (SourceConfig, JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let catalog_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler = move |Json(request): Json<Value>| {
            let gate = gate.clone();
            let catalog_reads = catalog_reads.clone();
            async move {
                let method = request["method"].as_str().unwrap_or("");
                if request.get("id").is_none() {
                    return StatusCode::ACCEPTED.into_response();
                }
                if method == "tools/call" {
                    if let Some((started, release)) = gate {
                        started.notify_one();
                        release.notified().await;
                    }
                }
                let result = match method {
                    "initialize" => {
                        json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{},"resources":{},"prompts":{},"logging":{}},"serverInfo":{"name":tag,"version":"1"}})
                    }
                    "tools/list" => {
                        let read = catalog_reads.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                        let visibility = if tag == "authority-changed" && read > 0 {
                            json!(["model"])
                        } else {
                            json!(["model", "app"])
                        };
                        json!({"tools":[{"name":"same_tool","inputSchema":{"type":"object"},"_meta":{"ui":{"resourceUri":"ui://fixture/app.html","visibility":visibility}}}]})
                    }
                    "tools/call" => {
                        json!({"content":[{"type":"text","text":tag}],"structuredContent":{"source":tag}})
                    }
                    "resources/read" => {
                        json!({"contents":[{"uri":"ui://fixture/app.html","mimeType":"text/html;profile=mcp-app","text":format!("<p>{tag}</p>")}]})
                    }
                    _ => json!({}),
                };
                if method == "resources/read" && fail_resource {
                    Json(
                        json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"fixture unavailable"}}),
                    ).into_response()
                } else {
                    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
                        .into_response()
                }
            }
        };
        let router = Router::new().route("/mcp", post(handler));
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {accepted=listener.accept()=> {let Ok((stream,_))=accepted else {break};let service=TowerToHyperService::new(router.clone());connections.spawn(async move {let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).await;});},Some(_)=connections.join_next(),if !connections.is_empty()=>{}}
            }
        });
        (
            SourceConfig {
                id: tag.into(),
                name: tag.into(),
                url,
            },
            task,
        )
    }

    #[tokio::test]
    async fn same_named_tools_bind_to_original_source_and_failed_resource_keeps_result() {
        let (first, first_task) = fixture_source("first", false, None).await;
        let (second, second_task) = fixture_source("second", true, None).await;
        let broker = AppBroker::start(vec![first, second], "<p>shell</p>".into())
            .await
            .unwrap();
        let run = Uuid::new_v4();
        broker.begin_turn(run).unwrap();
        for tag in ["first", "second"] {
            let source = broker.state.sources.get(tag).unwrap();
            let initialized = proxy_request(
                &broker.state,
                source,
                &json!({"method":"initialize","params":{"protocolVersion":"2025-11-25"}}),
            )
            .await
            .unwrap();
            assert_eq!(
                initialized["capabilities"],
                json!({"tools":{},"resources":{}})
            );
            let result=proxy_request(&broker.state,source,&json!({"method":"tools/call","params":{"name":"same_tool","arguments":{"tag":tag}}})).await.unwrap();
            assert_eq!(result["structuredContent"]["source"], tag);
        }
        let artifacts = broker.finish_turn(run).unwrap();
        assert_eq!(artifacts[0].server_id, "first");
        assert_eq!(artifacts[0].resource.html, "<p>first</p>");
        assert_eq!(artifacts[1].server_id, "second");
        assert_eq!(artifacts[1].result["structuredContent"]["source"], "second");
        assert!(artifacts[1].resource.html.is_empty());
        assert_eq!(artifacts[1].resource.meta["lens/resourceUnavailable"], true);
        broker.close().await;
        first_task.abort();
        second_task.abort();
    }

    #[tokio::test]
    async fn delayed_original_run_result_cannot_attach_to_replacement_run() {
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let (config, task) =
            fixture_source("delayed", false, Some((started.clone(), release.clone()))).await;
        let broker = AppBroker::start(vec![config], "<p>shell</p>".into())
            .await
            .unwrap();
        let old = Uuid::new_v4();
        broker.begin_turn(old).unwrap();
        let state = broker.state.clone();
        let source = state.sources["delayed"].clone();
        let pending = tokio::spawn(async move {
            proxy_request(&state,&source,&json!({"method":"tools/call","params":{"name":"same_tool","arguments":{"exact":"old"}}})).await
        });
        started.notified().await;
        broker.abort_turn(old);
        let replacement = Uuid::new_v4();
        broker.begin_turn(replacement).unwrap();
        release.notify_one();
        assert_eq!(
            pending.await.unwrap().unwrap()["structuredContent"]["source"],
            "delayed"
        );
        assert!(broker.finish_turn(replacement).unwrap().is_empty());
        broker.close().await;
        task.abort();
    }
    #[tokio::test]
    async fn app_visibility_is_refreshed_instead_of_reusing_initial_model_catalog() {
        let (config, task) = fixture_source("authority-changed", false, None).await;
        let broker = AppBroker::start(vec![config], "<p>shell</p>".into())
            .await
            .unwrap();
        let run = Uuid::new_v4();
        broker.begin_turn(run).unwrap();
        let source = &broker.state.sources["authority-changed"];
        proxy_request(
            &broker.state,
            source,
            &json!({"method":"tools/call","params":{"name":"same_tool","arguments":{}}}),
        )
        .await
        .unwrap();
        let artifacts = broker.finish_turn(run).unwrap();
        assert!(broker
            .call_from_app(&artifacts[0], json!({"name":"same_tool","arguments":{}}))
            .await
            .is_err());
        assert_eq!(
            broker.list_from_app(&artifacts[0]).await.unwrap(),
            json!({"tools":[]})
        );
        broker.close().await;
        task.abort();
    }
}
