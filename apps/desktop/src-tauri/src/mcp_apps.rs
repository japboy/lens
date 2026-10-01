//! Native authority for immutable MCP App artifacts and finite display leases.
use crate::{
    app_state::{emit_app_snapshot, AppState},
    model::{AppSnapshot, LensStage},
};
use adapter_output_mcp::apps::{AppArtifact, AppBroker, AppResource, DisplayServer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;
use usecase::model::{McpAppDescriptor, McpAppServer};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
struct RetainedArtifact {
    descriptor: McpAppDescriptor,
    artifact: AppArtifact,
}
#[derive(Clone, Serialize)]
pub struct AppDraft {
    pub id: Uuid,
    pub text: String,
}
#[derive(Clone, Serialize)]
pub struct AppLink {
    pub id: Uuid,
    pub url: String,
}
struct Lease {
    artifact: RetainedArtifact,
    source: Weak<AppBroker>,
    generation: Uuid,
    source_generation: Option<Uuid>,
    _document: DisplayServer,
    context: Value,
    draft: Option<AppDraft>,
    link: Option<AppLink>,
    submitted: bool,
    cancellation: CancellationToken,
}
struct SourceAuthority {
    operation_id: Uuid,
    session_id: String,
    generation: Uuid,
    broker: Weak<AppBroker>,
    config: crate::model::AppConfig,
}
#[derive(Default)]
struct ArtifactStorage {
    operation_id: Option<Uuid>,
    directory: Option<tempfile::TempDir>,
}

#[derive(Default)]
pub struct McpAppsStore {
    source: Mutex<Option<SourceAuthority>>,
    leases: Mutex<HashMap<Uuid, Lease>>,
    storage: Mutex<ArtifactStorage>,
    open_epoch: std::sync::atomic::AtomicU64,
    closed: std::sync::atomic::AtomicBool,
}
pub struct SourceLifetime {
    store: Arc<McpAppsStore>,
    generation: Uuid,
}
impl Drop for SourceLifetime {
    fn drop(&mut self) {
        if let Ok(mut source) = self.store.source.lock() {
            if source
                .as_ref()
                .is_some_and(|s| s.generation == self.generation)
            {
                *source = None;
            }
        }
        if let Ok(mut leases) = self.store.leases.lock() {
            leases.retain(|_, lease| {
                if lease.source_generation == Some(self.generation) {
                    lease.cancellation.cancel();
                    false
                } else {
                    true
                }
            });
        }
    }
}
#[derive(Serialize)]
pub struct OpenedApp {
    pub id: Uuid,
    pub artifact_id: Uuid,
    pub generation: Uuid,
    pub proxy_url: String,
    pub proxy_origin: String,
    pub resource: AppResource,
    pub input: Value,
    pub result: Value,
    pub host_capabilities: Value,
    pub live: bool,
    pub document_url: Option<String>,
}
impl OpenedApp {
    fn set_live(&mut self, live: bool) -> Result<(), String> {
        let sandbox = adapter_output_mcp::apps::resource_sandbox(&self.resource)?;
        self.live = live;
        self.host_capabilities = if live {
            json!({"serverTools":{},"message":{"text":{}},"updateModelContext":{"text":{},"structuredContent":{}}})
        } else {
            json!({})
        };
        self.host_capabilities["openLinks"] = json!({});
        self.host_capabilities["sandbox"] = sandbox;
        Ok(())
    }
}
#[derive(Serialize)]
pub struct AppRequestResult {
    pub result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<AppDraft>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<AppLink>,
}

impl McpAppsStore {
    fn ensure_open(&self) -> Result<(), String> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            Err("App Host is shutting down".into())
        } else {
            Ok(())
        }
    }

    /// Event-loop exit does not guarantee managed-state destructors run.
    /// Retire authority first, then explicitly release only this process's resources.
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.open_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        {
            let mut leases = self.leases.lock().map_err(|_| "MCP leases unavailable")?;
            for lease in leases.values() {
                lease.cancellation.cancel();
            }
            leases.clear();
        }
        *self.source.lock().map_err(|_| "MCP source unavailable")? = None;
        let directory = {
            let mut storage = self.storage.lock().map_err(|_| "MCP storage unavailable")?;
            storage.operation_id = None;
            storage.directory.take()
        };
        if let Some(directory) = directory {
            directory
                .close()
                .map_err(|_| "Unable to release App artifacts")?;
        }
        Ok(())
    }

    pub fn install(
        self: &Arc<Self>,
        operation_id: Uuid,
        session_id: String,
        broker: &Arc<AppBroker>,
        config: &crate::model::AppConfig,
    ) -> Result<SourceLifetime, String> {
        let generation = Uuid::new_v4();
        let mut source = self
            .source
            .lock()
            .map_err(|_| "MCP source state unavailable")?;
        self.ensure_open()?;
        *source = Some(SourceAuthority {
            operation_id,
            session_id,
            generation,
            broker: Arc::downgrade(broker),
            config: config.clone(),
        });
        Ok(SourceLifetime {
            store: Arc::clone(self),
            generation,
        })
    }

    pub(crate) fn sync_operation(&self, state: &AppState) -> Result<(), String> {
        {
            let mut storage = self.storage.lock().map_err(|_| "MCP storage unavailable")?;
            if self.ensure_open().is_err() {
                return Ok(());
            }
            // Read the authoritative operation while holding storage: a delayed publication
            // must never reset a newer operation's files using its stale snapshot.
            let runtime = state
                .runtime
                .read()
                .map_err(|_| "Application state unavailable")?;
            let operation_id = runtime.lens.operation_id;
            if storage.operation_id != operation_id {
                storage.directory = None;
                storage.operation_id = operation_id;
                self.open_epoch
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
        }
        let mut leases = self.leases.lock().map_err(|_| "MCP leases unavailable")?;
        let runtime = state
            .runtime
            .read()
            .map_err(|_| "Application state unavailable")?;
        let operation_id = runtime.lens.operation_id;
        leases.retain(|_, lease| {
            if Some(lease.artifact.descriptor.operation_id) != operation_id {
                lease.cancellation.cancel();
                false
            } else {
                if !self.lease_live(&runtime, lease) {
                    // Pause retires RPC authority, but preserves the read-only display.
                    lease.cancellation.cancel();
                    lease.draft = None;
                }
                true
            }
        });
        Ok(())
    }
    fn lease_live(&self, runtime: &AppSnapshot, lease: &Lease) -> bool {
        if self.ensure_open().is_err()
            || lease.cancellation.is_cancelled()
            || !operation_eligible(runtime, &lease.artifact.descriptor)
        {
            return false;
        }
        self.source.lock().ok().is_some_and(|source| {
            source.as_ref().is_some_and(|source| {
                Some(source.generation) == lease.source_generation
                    && source.operation_id == lease.artifact.descriptor.operation_id
                    && source.session_id == lease.artifact.descriptor.session_id
                    && source.config.same_active_session_config(&runtime.config)
                    && Weak::ptr_eq(&source.broker, &lease.source)
                    && lease.source.upgrade().is_some()
            })
        })
    }
    fn commit_display(
        &self,
        state: &AppState,
        ticket: u64,
        id: Uuid,
        lease: Lease,
    ) -> Result<bool, String> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "MCP lease state unavailable")?;
        self.ensure_open()?;
        if self.open_epoch.load(std::sync::atomic::Ordering::Acquire) != ticket {
            return Err("App display request was superseded".into());
        }
        let latest = state
            .runtime
            .read()
            .map_err(|_| "Application state unavailable")?;
        let descriptor = &lease.artifact.descriptor;
        if latest.lens.operation_id != Some(descriptor.operation_id)
            || !(latest.lens.mcp_apps.iter().any(|a| a.id == descriptor.id)
                || latest
                    .lens
                    .response_history
                    .responses
                    .iter()
                    .any(|r| r.mcp_apps.iter().any(|a| a.id == descriptor.id)))
        {
            return Err("App operation expired before display opened".into());
        }
        let live = self.lease_live(&latest, &lease);
        for old in leases.values() {
            old.cancellation.cancel();
        }
        leases.clear();
        leases.insert(id, lease);
        Ok(live)
    }
    fn artifact_path(&self, id: Uuid) -> Result<std::path::PathBuf, String> {
        let storage = self.storage.lock().map_err(|_| "MCP storage unavailable")?;
        Ok(storage
            .directory
            .as_ref()
            .ok_or("MCP artifact unavailable")?
            .path()
            .join(format!("{id}.json")))
    }
    pub(crate) fn discard(&self, operation: Uuid, descriptors: &[McpAppDescriptor]) {
        if let Ok(storage) = self.storage.lock() {
            if storage.operation_id == Some(operation) {
                if let Some(directory) = &storage.directory {
                    for descriptor in descriptors {
                        let _ = fs::remove_file(
                            directory.path().join(format!("{}.json", descriptor.id)),
                        );
                    }
                }
            }
        }
    }
    #[cfg(debug_assertions)]
    pub(crate) fn validation_snapshot(&self) -> Value {
        let source_live = self
            .source
            .lock()
            .ok()
            .is_some_and(|s| s.as_ref().is_some_and(|s| s.broker.upgrade().is_some()));
        let leases=self.leases.lock().map(|leases| leases.iter().map(|(id,l)| json!({"id":id,"artifact_id":l.artifact.descriptor.id,"generation":l.generation,"source_live":l.source.upgrade().is_some(),"draft_id":l.draft.as_ref().map(|d|d.id),"submitted":l.submitted,"authority_active":!l.cancellation.is_cancelled()})).collect::<Vec<_>>()).unwrap_or_default();
        json!({"source_live":source_live,"leases":leases})
    }
    fn is_live(&self, descriptor: &McpAppDescriptor) -> bool {
        self.source
            .lock()
            .ok()
            .and_then(|source| {
                source.as_ref().map(|s| {
                    s.operation_id == descriptor.operation_id
                        && s.session_id == descriptor.session_id
                        && s.broker.upgrade().is_some()
                })
            })
            .unwrap_or(false)
    }
    pub fn retain<R: tauri::Runtime>(
        &self,
        app: &AppHandle<R>,
        operation_id: Uuid,
        session_id: &str,
        artifacts: Vec<AppArtifact>,
    ) -> Result<Vec<McpAppDescriptor>, String> {
        let state = app.state::<AppState>();
        let mut storage = self.storage.lock().map_err(|_| "MCP storage unavailable")?;
        self.ensure_open()?;
        let runtime = state
            .runtime
            .read()
            .map_err(|_| "Application state unavailable")?;
        if runtime.lens.operation_id != Some(operation_id) {
            return Err("MCP artifact operation expired".into());
        }
        if artifacts.iter().any(|artifact| {
            runtime.lens.agent.as_ref().is_none_or(|run| {
                run.run_id != artifact.run_id || run.session_id.as_deref() != Some(session_id)
            })
        }) {
            return Err("MCP artifact run expired".into());
        }
        // Retain can precede the first publication of a new operation.
        if storage.operation_id != Some(operation_id) {
            storage.directory = None;
            storage.operation_id = Some(operation_id);
        }
        if storage.operation_id != Some(operation_id) {
            return Err("MCP artifact operation expired".into());
        }
        if storage.directory.is_none() {
            storage.directory = Some(
                tempfile::Builder::new()
                    .prefix("lens-mcp-apps-")
                    .tempdir()
                    .map_err(|_| "Unable to create owned App storage")?,
            );
        }
        let mut descriptors = Vec::new();
        let mut pending_files = PendingArtifacts::default();
        for artifact in artifacts {
            let descriptor = McpAppDescriptor {
                id: artifact.id,
                operation_id,
                session_id: session_id.into(),
                server_id: artifact.server_id.clone(),
                tool_name: artifact.tool_name.clone(),
                resource_uri: artifact.resource_uri.clone(),
                title: artifact.title.clone(),
                retained_bytes: serde_json::to_vec(&artifact)
                    .map_err(|_| "Unable to encode App artifact")?
                    .len(),
            };
            let retained = RetainedArtifact {
                descriptor: descriptor.clone(),
                artifact,
            };
            let path = storage
                .directory
                .as_ref()
                .expect("initialized directory")
                .path()
                .join(format!("{}.json", descriptor.id));
            let parent = path.parent().ok_or("Invalid MCP storage path")?;
            fs::create_dir_all(parent).map_err(|_| "Unable to create MCP artifact storage")?;
            let entries = fs::read_dir(parent)
                .map_err(|_| "Unable to inspect MCP storage")?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Unable to inspect MCP storage")?;
            let mut retained_bytes = 0u64;
            for entry in &entries {
                retained_bytes = retained_bytes.saturating_add(
                    entry
                        .metadata()
                        .map_err(|_| "Unable to inspect MCP artifact")?
                        .len(),
                );
            }
            if entries.len() >= 4096
                || retained_bytes.saturating_add(descriptor.retained_bytes as u64 + 4096)
                    > 256 * 1024 * 1024
            {
                return Err("MCP artifact storage is full; existing history was preserved".into());
            }
            let mut file = tempfile::NamedTempFile::new_in(parent)
                .map_err(|_| "Unable to create MCP artifact")?;
            file.write_all(
                &serde_json::to_vec(&retained).map_err(|_| "Unable to encode MCP artifact")?,
            )
            .map_err(|_| "Unable to save MCP artifact")?;
            file.as_file()
                .sync_all()
                .map_err(|_| "Unable to sync MCP artifact")?;
            file.persist_noclobber(&path)
                .map_err(|_| "MCP artifact already exists or storage failed")?;
            pending_files.paths.push(path);
            descriptors.push(descriptor);
        }
        pending_files.committed = true;
        Ok(descriptors)
    }
}
#[derive(Default)]
struct PendingArtifacts {
    paths: Vec<std::path::PathBuf>,
    committed: bool,
}
impl Drop for PendingArtifacts {
    fn drop(&mut self) {
        if !self.committed {
            for path in &self.paths {
                let _ = fs::remove_file(path);
            }
        }
    }
}
fn load<R: tauri::Runtime>(app: &AppHandle<R>, id: Uuid) -> Result<RetainedArtifact, String> {
    let state = app.state::<AppState>();
    let path = state.mcp_apps.artifact_path(id)?;
    let meta = fs::metadata(&path).map_err(|_| "MCP artifact unavailable")?;
    if meta.len() > (16 * 1024 * 1024) as u64 {
        return Err("Retained MCP artifact exceeds size limit".into());
    }
    let artifact: RetainedArtifact =
        serde_json::from_slice(&fs::read(path).map_err(|_| "Unable to read MCP artifact")?)
            .map_err(|_| "Invalid retained MCP artifact")?;
    if artifact.descriptor.id != id || artifact.artifact.id != id {
        return Err("MCP artifact identity mismatch".into());
    }
    Ok(artifact)
}
fn validate_resource(artifact: &AppArtifact) -> Result<(), String> {
    adapter_output_mcp::apps::resource_csp(&artifact.resource)?;
    if artifact.resource.html.is_empty() {
        return Err(
            "App resource could not be retained; the original tool result is preserved".into(),
        );
    }
    Ok(())
}
fn operation_eligible(runtime: &AppSnapshot, descriptor: &McpAppDescriptor) -> bool {
    runtime.lens.operation_id == Some(descriptor.operation_id)
        && runtime
            .lens
            .live
            .as_ref()
            .is_none_or(|live| live.lifecycle == crate::model::LensMonitoringLifecycle::Watching)
}
fn host_origin_matches(url: &reqwest::Url, origin: &str) -> bool {
    if url.origin().ascii_serialization() == origin {
        return true;
    }
    url.scheme() == "tauri" && url.host_str() == Some("localhost") && origin == "tauri://localhost"
}
fn known_host_url(url: &reqwest::Url, dev_url: Option<&reqwest::Url>) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    // Tauri 2.12 manager::tauri_protocol_url uses these exact embedded origins;
    // Wry maps the custom protocol to HTTP(S) on Windows/Android.
    let embedded = url.port().is_none()
        && match url.scheme() {
            "tauri" => url.host_str() == Some("localhost"),
            "http" | "https" => url.host_str() == Some("tauri.localhost"),
            _ => false,
        };
    embedded || dev_url.is_some_and(|dev| dev.origin() == url.origin())
}

#[tauri::command]
pub async fn open_mcp_app<R: tauri::Runtime>(
    app: AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    artifact_id: Uuid,
    host_origin: String,
) -> Result<OpenedApp, String> {
    let state = app.state::<AppState>();
    let open_ticket = state
        .mcp_apps
        .open_epoch
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
        + 1;
    let url = window.url().map_err(|_| "Unable to inspect Host URL")?;
    let known_host = known_host_url(&url, app.config().build.dev_url.as_ref());
    if window.label() != crate::ui::LENS_WINDOW_LABEL
        || !known_host
        || !host_origin_matches(&url, &host_origin)
    {
        return Err("App Host origin mismatch".into());
    }
    let artifact = load(&app, artifact_id)?;
    validate_resource(&artifact.artifact)?;
    let state = app.state::<AppState>();
    let snapshot = state.snapshot()?;
    let current = snapshot.lens.operation_id == Some(artifact.descriptor.operation_id)
        && snapshot.lens.mcp_apps.iter().any(|a| a.id == artifact_id);
    let known = current
        || snapshot
            .lens
            .response_history
            .responses
            .iter()
            .any(|r| r.mcp_apps.iter().any(|a| a.id == artifact_id));
    if !known {
        return Err("App artifact is not part of this Lens history".into());
    }
    let live = known
        && snapshot.lens.operation_id == Some(artifact.descriptor.operation_id)
        && state.mcp_apps.is_live(&artifact.descriptor)
        && snapshot
            .lens
            .live
            .as_ref()
            .is_none_or(|l| l.lifecycle == crate::model::LensMonitoringLifecycle::Watching);
    let generation = Uuid::new_v4();
    let id = Uuid::new_v4();
    let document = if artifact.artifact.server_id == adapter_output_mcp::apps::FALLBACK_SERVER {
        artifact
            .artifact
            .result
            .pointer("/structuredContent/html")
            .and_then(Value::as_str)
            .map(str::to_owned)
    } else {
        None
    };
    let display = DisplayServer::start(
        artifact.artifact.resource.clone(),
        document,
        host_origin,
        include_str!("../../src/mcp-apps/sandbox-proxy.html").into(),
        include_str!("../../src/mcp-apps/sandbox-proxy.js").into(),
    )
    .await
    .map_err(|_| "Unable to start App sandbox")?;
    let result = artifact.artifact.result.clone();
    // A Host-owned display location is separate from the immutable source tool result.
    // The SDK gets the original result; the bundled shell reads this Host-context value.
    let mut response = OpenedApp {
        id,
        artifact_id,
        generation,
        proxy_url: display.proxy_url.clone(),
        proxy_origin: display.origin.clone(),
        resource: artifact.artifact.resource.clone(),
        input: artifact.artifact.input.clone(),
        result,
        host_capabilities: json!({}),
        live,
        document_url: display.document_url.clone(),
    };
    let source = state
        .mcp_apps
        .source
        .lock()
        .map_err(|_| "MCP source state unavailable")?
        .as_ref()
        .filter(|s| {
            live && s.operation_id == artifact.descriptor.operation_id
                && s.session_id == artifact.descriptor.session_id
        })
        .map(|s| (s.broker.clone(), Some(s.generation)))
        .unwrap_or_default();
    let (source, source_generation) = source;
    let committed_live = state.mcp_apps.commit_display(
        &state,
        open_ticket,
        id,
        Lease {
            artifact,
            source,
            generation,
            source_generation,
            _document: display,
            context: json!({}),
            draft: None,
            link: None,
            submitted: false,
            cancellation: CancellationToken::new(),
        },
    )?;
    response.set_live(committed_live)?;
    Ok(response)
}
#[tauri::command]
pub fn close_mcp_app<R: tauri::Runtime>(app: AppHandle<R>, lease_id: Uuid) -> Result<(), String> {
    let state = app.state::<AppState>();
    if let Some(lease) = state
        .mcp_apps
        .leases
        .lock()
        .map_err(|_| "MCP lease state unavailable")?
        .remove(&lease_id)
    {
        lease.cancellation.cancel();
    }
    Ok(())
}

fn ensure_display(
    store: &McpAppsStore,
    snapshot: &AppSnapshot,
    lease: &Lease,
) -> Result<(), String> {
    store.ensure_open()?;
    let descriptor = &lease.artifact.descriptor;
    if snapshot.lens.operation_id != Some(descriptor.operation_id)
        || !(snapshot
            .lens
            .mcp_apps
            .iter()
            .any(|app| app.id == descriptor.id)
            || snapshot
                .lens
                .response_history
                .responses
                .iter()
                .any(|response| response.mcp_apps.iter().any(|app| app.id == descriptor.id)))
    {
        return Err("App display expired".into());
    }
    Ok(())
}
fn link_destination(params: &Value) -> Result<String, String> {
    let destination = params
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.len() <= 4096)
        .ok_or("Invalid App link")?;
    let url = reqwest::Url::parse(destination).map_err(|_| "Invalid App link")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Only credential-free HTTP(S) links are supported".into());
    }
    Ok(url.to_string())
}
fn ensure_host<R: tauri::Runtime>(
    app: &AppHandle<R>,
    window: &tauri::WebviewWindow<R>,
) -> Result<(), String> {
    let url = window.url().map_err(|_| "Unable to inspect Host URL")?;
    if window.label() != crate::ui::LENS_WINDOW_LABEL
        || !known_host_url(&url, app.config().build.dev_url.as_ref())
    {
        return Err("App Host mismatch".into());
    }
    Ok(())
}
#[tauri::command]
pub fn prepare_mcp_app_document<R: tauri::Runtime>(
    app: AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    lease_id: Uuid,
    document: String,
) -> Result<(), String> {
    ensure_host(&app, &window)?;
    let state = app.state::<AppState>();
    let leases = state
        .mcp_apps
        .leases
        .lock()
        .map_err(|_| "MCP leases unavailable")?;
    let lease = leases.get(&lease_id).ok_or("App lease expired")?;
    ensure_display(&state.mcp_apps, &state.snapshot()?, lease)?;
    if lease.artifact.artifact.server_id != adapter_output_mcp::apps::FALLBACK_SERVER {
        return Err("Only bundled HTML documents can be prepared".into());
    }
    lease._document.prepare_document(document)
}
#[tauri::command]
pub fn submit_mcp_app_link<R: tauri::Runtime>(
    app: AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    lease_id: Uuid,
    link_id: Uuid,
) -> Result<(), String> {
    ensure_host(&app, &window)?;
    let state = app.state::<AppState>();
    let destination = {
        let mut leases = state
            .mcp_apps
            .leases
            .lock()
            .map_err(|_| "MCP leases unavailable")?;
        let lease = leases.get_mut(&lease_id).ok_or("App lease expired")?;
        ensure_display(&state.mcp_apps, &state.snapshot()?, lease)?;
        if lease.link.as_ref().is_none_or(|link| link.id != link_id) {
            return Err("App link was replaced or already opened".into());
        }
        lease.link.take().expect("checked link").url
    };
    app.opener()
        .open_url(destination, None::<&str>)
        .map_err(|_| "Unable to open App link in default browser".into())
}

fn bounded(value: &Value, max: usize) -> Result<(), String> {
    if serde_json::to_vec(value)
        .map_err(|_| "Invalid App request")?
        .len()
        > max
    {
        return Err("App request exceeds size limit".into());
    }
    Ok(())
}
fn text_content(params: &Value) -> Result<String, String> {
    if params.get("role").and_then(Value::as_str) != Some("user") {
        return Err("Only user messages are accepted".into());
    }
    let content = params
        .get("content")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty() && v.len() <= 16)
        .ok_or("App message must contain text")?;
    let text = content
        .iter()
        .map(|v| {
            if v.get("type").and_then(Value::as_str) == Some("text") {
                v.get("text")
                    .and_then(Value::as_str)
                    .ok_or("Invalid App text")
            } else {
                Err("Only text App messages are supported")
            }
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");
    if text.trim().is_empty() {
        return Err("App message must contain text".into());
    }
    Ok(text)
}
#[tauri::command]
pub async fn mcp_app_request<R: tauri::Runtime>(
    app: AppHandle<R>,
    lease_id: Uuid,
    request: Value,
) -> Result<AppRequestResult, String> {
    bounded(&request, 64 * 1024)?;
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or("Invalid App method")?;
    let params = request.get("params").cloned().unwrap_or(json!({}));
    let state = app.state::<AppState>();
    let (artifact, source, generation, cancellation) = {
        let mut leases = state
            .mcp_apps
            .leases
            .lock()
            .map_err(|_| "MCP lease state unavailable")?;
        let lease = leases.get_mut(&lease_id).ok_or("App lease expired")?;
        if method == "ui/open-link" {
            ensure_display(&state.mcp_apps, &state.snapshot()?, lease)?;
            let url = link_destination(&params)?;
            let link = AppLink {
                id: Uuid::new_v4(),
                url,
            };
            lease.link = Some(link.clone());
            return Ok(AppRequestResult {
                result: json!({"isError":false}),
                draft: None,
                link: Some(link),
            });
        }
        if !state.mcp_apps.lease_live(&state.snapshot()?, lease)
            || state.lens()?.operation_id != Some(lease.artifact.descriptor.operation_id)
        {
            return Err("App session authority expired".into());
        }
        match method {
            "ui/update-model-context" => {
                bounded(&params, 32 * 1024)?;
                if !params.is_object()
                    || params
                        .get("structuredContent")
                        .is_some_and(|v| !v.is_object())
                {
                    return Err("App model context must be an object".into());
                }
                if let Some(content) = params.get("content") {
                    let blocks = content
                        .as_array()
                        .filter(|v| v.len() <= 16)
                        .ok_or("Invalid App context content")?;
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) != Some("text")
                            || block.get("text").and_then(Value::as_str).is_none()
                        {
                            return Err("Only text App context is supported".into());
                        }
                    }
                }
                lease.context = params;
                return Ok(AppRequestResult {
                    result: json!({}),
                    draft: None,
                    link: None,
                });
            }
            "ui/message" => {
                let text = text_content(&params)?;
                bounded(&params, 16 * 1024)?;
                let draft = AppDraft {
                    id: Uuid::new_v4(),
                    text,
                };
                lease.draft = Some(draft.clone());
                lease.submitted = false;
                return Ok(AppRequestResult {
                    result: json!({}),
                    draft: Some(draft),
                    link: None,
                });
            }
            "tools/call" | "tools/list" => {}
            _ => return Err("Unsupported App operation".into()),
        }
        if state.lens()?.stage == LensStage::Transforming
            || state.agent_control.has_pending_work()?
        {
            return Err("Agent is busy; retry the App operation when it finishes".into());
        }
        (
            lease.artifact.artifact.clone(),
            lease.source.upgrade().ok_or("MCP source unavailable")?,
            lease.generation,
            lease.cancellation.clone(),
        )
    };
    let operation = async {
        if method == "tools/list" {
            source.list_from_app(&artifact).await
        } else {
            source.call_from_app(&artifact, params).await
        }
    };
    let result = tokio::select! {_=cancellation.cancelled()=>return Err("App lease expired".into()),result=tokio::time::timeout(Duration::from_secs(30),operation)=>result.map_err(|_|"App tool request timed out")??};
    let leases = state
        .mcp_apps
        .leases
        .lock()
        .map_err(|_| "MCP lease state unavailable")?;
    let runtime = state
        .runtime
        .read()
        .map_err(|_| "Application state unavailable")?;
    if leases
        .get(&lease_id)
        .is_none_or(|l| l.generation != generation || !state.mcp_apps.lease_live(&runtime, l))
        || cancellation.is_cancelled()
    {
        return Err("App lease expired before the tool result returned".into());
    }
    Ok(AppRequestResult {
        result,
        draft: None,
        link: None,
    })
}
#[tauri::command]
pub async fn submit_mcp_app_message<R: tauri::Runtime>(
    app: AppHandle<R>,
    lease_id: Uuid,
    draft_id: Uuid,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let receiver = {
        let mut leases = state
            .mcp_apps
            .leases
            .lock()
            .map_err(|_| "MCP lease state unavailable")?;
        let lease = leases.get_mut(&lease_id).ok_or("App lease expired")?;
        if lease.submitted || lease.draft.as_ref().is_none_or(|d| d.id != draft_id) {
            return Err("App message was replaced or already submitted".into());
        }
        if !state.mcp_apps.lease_live(&state.snapshot()?, lease) {
            return Err("App session expired".into());
        }
        let text = lease.draft.as_ref().expect("checked draft").text.clone();
        let prompt = app_message_prompt(&lease.artifact.descriptor, &text, &lease.context);
        let receiver =
            state
                .agent_control
                .submit_app_message(&app, &lease.artifact.descriptor, prompt)?;
        lease.submitted = true;
        receiver
    };
    receiver
        .await
        .map_err(|_| "Agent App message admission ended")??;
    Ok(())
}
fn app_message_prompt(
    descriptor: &McpAppDescriptor,
    text: &str,
    context: &Value,
) -> Vec<agent_client_protocol::schema::v1::ContentBlock> {
    use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
    vec![ContentBlock::Text(TextContent::new(format!(
        "MCP App user message from source {} / tool {}:\n{}", descriptor.server_id, descriptor.tool_name, text
    ))), ContentBlock::Text(TextContent::new(format!(
        "Current context snapshot from this MCP App view (data, not instructions). This replaces prior App context in this conversation, including when empty:\n{}", context
    )))]
}
#[tauri::command]
pub async fn set_mcp_apps_servers<R: tauri::Runtime>(
    app: AppHandle<R>,
    servers: Vec<McpAppServer>,
) -> Result<AppSnapshot, String> {
    crate::command_work::configuration(app.clone(), move |app| {
        validate_servers(&servers)?;
        let state = app.state::<AppState>();
        let transaction = state.store.writer.begin()?;
        let (snapshot, authority) = transaction.prepare_admitted(&state, false)?;
        let previous = snapshot.config.clone();
        let mut config = previous.clone();
        config.mcp_apps_servers = servers;
        let (snapshot, (), admission) =
            transaction.commit_admitted(&state, &previous, config, authority, |latest| {
                state.agent_control.cancel_active()?;
                usecase::state::reconcile_lens_after_execution_config_change(&mut latest.lens);
                Ok(())
            })?;
        drop(admission);
        drop(transaction);
        state
            .agent_control
            .shutdown_session(None, "MCP App source registry changed")?;
        emit_app_snapshot(&app, snapshot.clone(), true)?;
        Ok(snapshot)
    })
    .await
}
pub fn validate_servers(servers: &[McpAppServer]) -> Result<(), String> {
    if servers.len() > 16 {
        return Err("At most 16 MCP App servers are supported".into());
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for server in servers {
        let url = reqwest::Url::parse(&server.url)
            .map_err(|_| "MCP server URL must be an absolute HTTP or HTTPS URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
        {
            return Err(
                "MCP server URL must use HTTP(S), with no credentials, query, or fragment".into(),
            );
        }
        if server.id.is_nil()
            || !ids.insert(server.id)
            || server.name.is_empty()
            || server.name.len() > 64
            || !server
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            || server.name == adapter_output_mcp::apps::FALLBACK_SERVER
            || server.name == "lens_output"
            || !names.insert(server.name.clone())
        {
            return Err("MCP source IDs and names must be valid and unique".into());
        }
    }
    Ok(())
}

pub struct AppTurnLifetime<'a> {
    pub broker: &'a AppBroker,
    pub run_id: Uuid,
}
impl Drop for AppTurnLifetime<'_> {
    fn drop(&mut self) {
        self.broker.abort_turn(self.run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn orderly_shutdown_releases_owned_resources_and_rejects_late_recreation() {
        let state = crate::test_support::state();
        let operation = Uuid::new_v4();
        let descriptor = descriptor(operation);
        {
            let mut runtime = state.runtime.write().unwrap();
            runtime.lens.operation_id = Some(operation);
            runtime.lens.mcp_apps = vec![descriptor.clone()];
        }
        let store = Arc::clone(&state.mcp_apps);
        let broker = Arc::new(
            AppBroker::start(Vec::new(), "<p>shell</p>".into())
                .await
                .unwrap(),
        );
        let config = state.config().unwrap();
        let lifetime = store
            .install(operation, descriptor.session_id.clone(), &broker, &config)
            .unwrap();
        store.sync_operation(&state).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let owned_path = directory.path().join("artifact.json");
        fs::write(&owned_path, b"owned").unwrap();
        store.storage.lock().unwrap().directory = Some(directory);
        let unowned = tempfile::tempdir().unwrap();
        let unowned_path = unowned.path().join("another-owner.json");
        fs::write(&unowned_path, b"unowned").unwrap();
        let display = lease(
            descriptor.clone(),
            Arc::downgrade(&broker),
            Some(lifetime.generation),
        )
        .await;
        let endpoint = display._document.proxy_url.clone();
        let cancellation = display.cancellation.clone();
        let ticket = store
            .open_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        assert!(store
            .commit_display(&state, ticket, Uuid::new_v4(), display)
            .unwrap());
        assert!(reqwest::Client::new()
            .get(&endpoint)
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        store.shutdown().unwrap();
        store.shutdown().unwrap();
        assert!(cancellation.is_cancelled());
        assert!(!owned_path.exists());
        assert!(unowned_path.exists());
        assert!(store.leases.lock().unwrap().is_empty());
        assert!(store.source.lock().unwrap().is_none());
        assert!(store
            .install(operation, descriptor.session_id.clone(), &broker, &config)
            .is_err());
        store.sync_operation(&state).unwrap();
        assert!(store.storage.lock().unwrap().directory.is_none());
        let delayed = lease(
            descriptor,
            Arc::downgrade(&broker),
            Some(lifetime.generation),
        )
        .await;
        assert!(store
            .commit_display(&state, ticket, Uuid::new_v4(), delayed)
            .is_err());
        tokio::task::yield_now().await;
        assert!(reqwest::Client::new().get(endpoint).send().await.is_err());
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(crate::product_context())
            .unwrap();
        assert!(store
            .retain(app.handle(), operation, "same-physical-session", Vec::new())
            .is_err());
        assert!(store.storage.lock().unwrap().directory.is_none());
        drop(lifetime);
    }
    #[test]
    fn link_requests_are_bounded_and_never_accept_native_or_credential_destinations() {
        assert_eq!(
            link_destination(&json!({"url":"https://example.com/path?q=1#x"})).unwrap(),
            "https://example.com/path?q=1#x"
        );
        for url in [
            "javascript:alert(1)",
            "file:///tmp/test.html",
            "tauri://localhost",
            "data:text/html,hi",
            "https://user:secret@example.com/",
            "mailto:hello@example.com",
        ] {
            assert!(link_destination(&json!({"url":url})).is_err(), "{url}");
        }
        assert!(link_destination(
            &json!({"url":format!("https://example.com/{}","x".repeat(4096))})
        )
        .is_err());
    }
    #[test]
    fn opened_app_serializes_only_implemented_live_content_modalities() {
        let mut opened = OpenedApp {
            id: Uuid::new_v4(),
            artifact_id: Uuid::new_v4(),
            generation: Uuid::new_v4(),
            proxy_url: "http://127.0.0.1:1234/proxy/test".into(),
            proxy_origin: "http://127.0.0.1:1234".into(),
            resource: AppResource {
                html: "<p>App</p>".into(),
                meta: json!({}),
            },
            input: json!({}),
            result: json!({}),
            host_capabilities: json!({}),
            live: false,
            document_url: None,
        };
        // Exercise the same final authority-to-DTO transition as open_mcp_app.
        opened.set_live(true).unwrap();
        let serialized = serde_json::to_value(&opened).unwrap();
        assert_eq!(serialized["live"], true);
        assert_eq!(
            serialized["host_capabilities"],
            json!({
                "serverTools": {},
                "openLinks": {},
                "message": {"text": {}},
                "updateModelContext": {"text": {}, "structuredContent": {}},
                "sandbox": {"csp": {"resourceDomains": [], "connectDomains": []}, "permissions": {}}
            })
        );
        opened.set_live(false).unwrap();
        let serialized = serde_json::to_value(&opened).unwrap();
        assert_eq!(serialized["live"], false);
        assert_eq!(
            serialized["host_capabilities"],
            json!({"openLinks":{},"sandbox":{"csp":{"resourceDomains":[],"connectDomains":[]},"permissions":{}}})
        );
        opened.resource.meta = json!({"ui":{"csp":{
            "resourceDomains":["https://cdn.example/", "https://*.assets.example"],
            "connectDomains":["wss://api.example/", "https://api.example:8443"]
        }}});
        opened.set_live(false).unwrap();
        let serialized = serde_json::to_value(&opened).unwrap();
        assert_eq!(
            serialized["host_capabilities"],
            json!({"openLinks":{},"sandbox":{"csp":{
            "resourceDomains":["https://cdn.example", "https://*.assets.example"],
            "connectDomains":["wss://api.example", "https://api.example:8443"]
        },"permissions":{}}})
        );
        let policy = adapter_output_mcp::apps::resource_csp(&opened.resource).unwrap();
        for domain in [
            "https://cdn.example",
            "https://*.assets.example",
            "wss://api.example",
            "https://api.example:8443",
        ] {
            assert!(policy.contains(domain));
        }
        opened.resource.meta =
            json!({"ui":{"csp":{"resourceDomains":["https://cdn.example; script-src *"]}}});
        assert!(opened.set_live(true).is_err());
    }
    fn descriptor(operation: Uuid) -> McpAppDescriptor {
        McpAppDescriptor {
            id: Uuid::new_v4(),
            operation_id: operation,
            session_id: "same-physical-session".into(),
            server_id: "server".into(),
            tool_name: "tool".into(),
            resource_uri: "ui://test/app.html".into(),
            title: "Test".into(),
            retained_bytes: 64,
        }
    }
    async fn lease(
        descriptor: McpAppDescriptor,
        source: Weak<AppBroker>,
        source_generation: Option<Uuid>,
    ) -> Lease {
        let resource = AppResource {
            html: "<p>App</p>".into(),
            meta: json!({}),
        };
        Lease {
            artifact: RetainedArtifact {
                artifact: AppArtifact {
                    id: descriptor.id,
                    run_id: Uuid::new_v4(),
                    server_id: descriptor.server_id.clone(),
                    tool_name: descriptor.tool_name.clone(),
                    resource_uri: descriptor.resource_uri.clone(),
                    title: descriptor.title.clone(),
                    resource: resource.clone(),
                    input: json!({}),
                    result: json!({}),
                },
                descriptor,
            },
            source,
            generation: Uuid::new_v4(),
            source_generation,
            _document: DisplayServer::start(
                resource,
                None,
                "tauri://localhost".into(),
                "__SETTINGS_JSON__".into(),
                String::new(),
            )
            .await
            .unwrap(),
            context: json!({}),
            draft: None,
            link: None,
            submitted: false,
            cancellation: CancellationToken::new(),
        }
    }
    #[tokio::test]
    async fn delayed_old_source_drop_cannot_revoke_new_source_lease() {
        let store = Arc::new(McpAppsStore::default());
        let broker = Arc::new(
            AppBroker::start(Vec::new(), "<p>shell</p>".into())
                .await
                .unwrap(),
        );
        let operation = Uuid::new_v4();
        let old = store
            .install(
                operation,
                "old-session".into(),
                &broker,
                &crate::model::AppConfig::new(std::path::PathBuf::from("/tmp")),
            )
            .unwrap();
        let newer = store
            .install(
                operation,
                "same-physical-session".into(),
                &broker,
                &crate::model::AppConfig::new(std::path::PathBuf::from("/tmp")),
            )
            .unwrap();
        let new_id = Uuid::new_v4();
        let new_lease = lease(
            descriptor(operation),
            Arc::downgrade(&broker),
            Some(newer.generation),
        )
        .await;
        let cancellation = new_lease.cancellation.clone();
        store.leases.lock().unwrap().insert(new_id, new_lease);
        drop(old);
        assert!(store.leases.lock().unwrap().contains_key(&new_id));
        assert!(!cancellation.is_cancelled());
        assert_eq!(
            store.source.lock().unwrap().as_ref().unwrap().generation,
            newer.generation
        );
        drop(newer);
        assert!(store.leases.lock().unwrap().is_empty());
        assert!(cancellation.is_cancelled());
    }
    #[tokio::test]
    async fn older_async_display_completion_cannot_replace_newer_display() {
        let state = crate::test_support::state();
        let operation = Uuid::new_v4();
        let first = descriptor(operation);
        let second = descriptor(operation);
        {
            let mut runtime = state.runtime.write().unwrap();
            runtime.lens.operation_id = Some(operation);
            runtime.lens.mcp_apps = vec![first.clone(), second.clone()];
        }
        let store = &state.mcp_apps;
        store.sync_operation(&state).unwrap();
        let old_ticket = store
            .open_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        // Both displays own real listeners; the second async operation commits first.
        let old = lease(first, Weak::new(), None).await;
        let old_endpoint = old._document.proxy_url.clone();
        let new_ticket = store
            .open_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        let new = lease(second, Weak::new(), None).await;
        let new_id = Uuid::new_v4();
        let new_cancel = new.cancellation.clone();
        assert!(!store
            .commit_display(&state, new_ticket, new_id, new)
            .unwrap());
        assert!(store
            .commit_display(&state, old_ticket, Uuid::new_v4(), old)
            .is_err());
        assert!(store.leases.lock().unwrap().contains_key(&new_id));
        assert!(!new_cancel.is_cancelled());
        tokio::task::yield_now().await;
        assert!(reqwest::Client::new()
            .get(old_endpoint)
            .send()
            .await
            .is_err());
    }
    #[test]
    fn operation_owned_files_survive_pause_and_are_removed_on_stop() {
        let state = crate::test_support::state();
        let operation = Uuid::new_v4();
        state.runtime.write().unwrap().lens.operation_id = Some(operation);
        state.mcp_apps.sync_operation(&state).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("immutable.json");
        fs::write(&path, b"retained").unwrap();
        state.mcp_apps.storage.lock().unwrap().directory = Some(directory);
        state.mcp_apps.sync_operation(&state).unwrap();
        assert!(path.exists());
        state.runtime.write().unwrap().lens.operation_id = None;
        state.mcp_apps.sync_operation(&state).unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn immutable_write_failure_rolls_back_only_owned_new_files() {
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().join("existing.json");
        let pending = directory.path().join("pending.json");
        fs::write(&existing, b"old").unwrap();
        fs::write(&pending, b"new").unwrap();
        drop(PendingArtifacts {
            paths: vec![pending.clone()],
            committed: false,
        });
        assert!(existing.exists());
        assert!(!pending.exists());
    }
    #[test]
    fn unavailable_resource_and_untrusted_origin_are_explicit() {
        assert!(host_origin_matches(
            &"tauri://localhost/".parse().unwrap(),
            "tauri://localhost"
        ));
        assert!(!host_origin_matches(
            &"tauri://localhost/".parse().unwrap(),
            "https://untrusted.example"
        ));
        let mut artifact = AppArtifact {
            id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            server_id: "source".into(),
            tool_name: "tool".into(),
            resource_uri: "ui://app".into(),
            title: "App".into(),
            resource: AppResource {
                html: String::new(),
                meta: json!({"lens/resourceUnavailable":true}),
            },
            input: json!({"exact":1}),
            result: json!({"content":[{"type":"text","text":"successful original effect"}]}),
        };
        let original = artifact.result.clone();
        assert!(validate_resource(&artifact).is_err());
        assert_eq!(artifact.result, original);
        artifact.resource.html = "<p>available</p>".into();
        assert!(validate_resource(&artifact).is_ok());
    }
    #[test]
    fn host_url_authority_matches_only_tauri_embedded_or_configured_dev_origins() {
        for owned in [
            "tauri://localhost/overlay.html",
            "http://tauri.localhost/overlay.html",
            "https://tauri.localhost/overlay.html",
        ] {
            assert!(known_host_url(&owned.parse().unwrap(), None));
        }
        for unowned in [
            "https://untrusted.example/",
            "http://tauri.localhost.evil/",
            "http://tauri.localhost:8080/",
            "tauri://localhost:99/",
            "http://user:secret@tauri.localhost/",
            "tauri://user@localhost/",
        ] {
            assert!(!known_host_url(&unowned.parse().unwrap(), None));
        }
        let dev = "http://127.0.0.1:1420/".parse().unwrap();
        assert!(known_host_url(
            &"http://127.0.0.1:1420/overlay".parse().unwrap(),
            Some(&dev)
        ));
        assert!(!known_host_url(
            &"http://127.0.0.1:1421/overlay".parse().unwrap(),
            Some(&dev)
        ));
    }
    #[test]
    fn every_followup_carries_current_context_even_after_explicit_clear() {
        let descriptor = descriptor(Uuid::new_v4());
        let old = serde_json::to_value(app_message_prompt(
            &descriptor,
            "explain",
            &json!({"structuredContent":{"selected_value":73}}),
        ))
        .unwrap();
        assert!(old[1]["text"].as_str().unwrap().contains("73"));
        let cleared =
            serde_json::to_value(app_message_prompt(&descriptor, "explain", &json!({}))).unwrap();
        let context = cleared[1]["text"].as_str().unwrap();
        assert!(context.contains("replaces prior App context"));
        assert!(context.ends_with("\n{}"));
        assert!(!context.contains("73"));
        assert_eq!(cleared[0]["text"], old[0]["text"]);
    }
    #[tokio::test]
    async fn pause_revokes_all_rpc_and_pending_authority_but_keeps_read_only_display() {
        let state = crate::test_support::state();
        let operation = Uuid::new_v4();
        let descriptor = descriptor(operation);
        {
            let mut runtime = state.runtime.write().unwrap();
            runtime.lens.operation_id = Some(operation);
            runtime.lens.mcp_apps = vec![descriptor.clone()];
            runtime.lens.live = Some(crate::model::LensLiveState {
                lifecycle: crate::model::LensMonitoringLifecycle::Watching,
                health: crate::model::LensSourceHealth::Healthy,
                freshness: crate::model::LensFreshness::Current,
                agent_refresh_interval_seconds: 180,
                last_outcome: None,
                error: None,
            });
        }
        let broker = Arc::new(
            AppBroker::start(Vec::new(), "<p>shell</p>".into())
                .await
                .unwrap(),
        );
        let lifetime = state
            .mcp_apps
            .install(
                operation,
                descriptor.session_id.clone(),
                &broker,
                &state.config().unwrap(),
            )
            .unwrap();
        state.mcp_apps.sync_operation(&state).unwrap();
        let mut display = lease(
            descriptor.clone(),
            Arc::downgrade(&broker),
            Some(lifetime.generation),
        )
        .await;
        display.artifact.artifact.server_id = adapter_output_mcp::apps::FALLBACK_SERVER.into();
        let endpoint = display._document.proxy_url.clone();
        let cancel = display.cancellation.clone();
        let id = Uuid::new_v4();
        let ticket = state
            .mcp_apps
            .open_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        assert!(state
            .mcp_apps
            .commit_display(&state, ticket, id, display)
            .unwrap());
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(crate::product_context())
            .unwrap();
        let handle = app.handle().clone();
        assert_eq!(
            mcp_app_request(
                handle.clone(),
                id,
                json!({"method":"tools/list","params":{}})
            )
            .await
            .unwrap()
            .result,
            json!({"tools":[]})
        );
        let pending = tokio::spawn({
            let cancel = cancel.clone();
            async move { cancel.cancelled().await }
        });
        {
            let state = app.state::<AppState>();
            state
                .runtime
                .write()
                .unwrap()
                .lens
                .live
                .as_mut()
                .unwrap()
                .lifecycle = crate::model::LensMonitoringLifecycle::Paused;
            state.mcp_apps.sync_operation(&state).unwrap();
        }
        tokio::time::timeout(Duration::from_millis(100), pending)
            .await
            .unwrap()
            .unwrap();
        assert!(cancel.is_cancelled());
        assert!(reqwest::Client::new()
            .get(&endpoint)
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
        let first_link = mcp_app_request(
            handle.clone(),
            id,
            json!({"method":"ui/open-link","params":{"url":"https://example.com/first"}}),
        )
        .await
        .unwrap()
        .link
        .unwrap();
        let second_link = mcp_app_request(
            handle.clone(),
            id,
            json!({"method":"ui/open-link","params":{"url":"https://example.com/second"}}),
        )
        .await
        .unwrap()
        .link
        .unwrap();
        assert_ne!(first_link.id, second_link.id);
        assert_eq!(
            app.state::<AppState>()
                .mcp_apps
                .leases
                .lock()
                .unwrap()
                .get(&id)
                .unwrap()
                .link
                .as_ref()
                .unwrap()
                .id,
            second_link.id
        );
        for method in [
            "tools/list",
            "tools/call",
            "ui/message",
            "ui/update-model-context",
        ] {
            let rejected =
                mcp_app_request(handle.clone(), id, json!({"method":method,"params":{}})).await;
            assert!(matches!(rejected,Err(error) if error.contains("authority expired")));
        }
        {
            let state = app.state::<AppState>();
            state
                .runtime
                .write()
                .unwrap()
                .lens
                .live
                .as_mut()
                .unwrap()
                .lifecycle = crate::model::LensMonitoringLifecycle::Watching;
            state.mcp_apps.sync_operation(&state).unwrap();
        }
        assert!(mcp_app_request(handle.clone(),id,json!({"method":"ui/message","params":{"role":"user","content":[{"type":"text","text":"late"}]}})).await.is_err());
        close_mcp_app(handle.clone(), id).unwrap();
        assert!(mcp_app_request(
            handle,
            id,
            json!({"method":"ui/open-link","params":{"url":"https://example.com/"}})
        )
        .await
        .is_err());
        tokio::task::yield_now().await;
        assert!(reqwest::Client::new().get(endpoint).send().await.is_err());
        drop(lifetime);
    }
    #[tokio::test]
    async fn captured_source_config_rejects_requests_before_old_source_finishes_shutdown() {
        let state = crate::test_support::state();
        let operation = Uuid::new_v4();
        let descriptor = descriptor(operation);
        state.runtime.write().unwrap().lens.operation_id = Some(operation);
        let broker = Arc::new(
            AppBroker::start(Vec::new(), "<p>shell</p>".into())
                .await
                .unwrap(),
        );
        let lifetime = state
            .mcp_apps
            .install(
                operation,
                descriptor.session_id.clone(),
                &broker,
                &state.config().unwrap(),
            )
            .unwrap();
        let display = lease(
            descriptor,
            Arc::downgrade(&broker),
            Some(lifetime.generation),
        )
        .await;
        assert!(state
            .mcp_apps
            .lease_live(&state.snapshot().unwrap(), &display));
        state.runtime.write().unwrap().config.working_directory =
            std::path::PathBuf::from("/different-session-cwd");
        assert!(!state
            .mcp_apps
            .lease_live(&state.snapshot().unwrap(), &display));
        assert!(
            state.mcp_apps.is_live(&display.artifact.descriptor),
            "old source is still registered while its captured configuration is retired"
        );
        drop(lifetime);
    }
}
