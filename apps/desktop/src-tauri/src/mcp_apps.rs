//! Native authority for immutable MCP App artifacts and finite display leases.
use crate::{
    app_state::{emit_app_snapshot, AppState},
    model::{AppSnapshot, LensStage},
};
use adapter_mcp_server::apps::{AppArtifact, AppBroker, AppResource, DisplayServer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio_util::sync::CancellationToken;
pub use usecase::mcp_apps::validate_servers;
use usecase::mcp_apps::{
    app_message_prompt, bounded, AppAuthorityFacts, AppViewInput, SourceAuthorityFacts,
};
use usecase::model::{McpAppDescriptor, McpAppServer};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
struct RetainedArtifact {
    descriptor: McpAppDescriptor,
    artifact: AppArtifact,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HtmlOutputRef {
    operation_id: Uuid,
    representation_id: Uuid,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HtmlPresentationSource {
    Live {
        output_ref: HtmlOutputRef,
        block_index: usize,
    },
    History {
        generation: Uuid,
        entry_id: String,
        revision: u64,
        block_index: usize,
    },
}
enum DisplayContent {
    Mcp(Box<RetainedArtifact>),
    Html {
        source: HtmlPresentationSource,
        mode: usecase::session_document::HtmlMode,
    },
}
impl DisplayContent {
    fn artifact(&self) -> Option<&RetainedArtifact> {
        match self {
            Self::Mcp(artifact) => Some(artifact),
            Self::Html { .. } => None,
        }
    }
    fn document_mode(&self) -> Option<usecase::session_document::HtmlMode> {
        match self {
            Self::Mcp(artifact) => (artifact.artifact.server_id
                == adapter_mcp_server::apps::FALLBACK_SERVER)
                .then_some(usecase::session_document::HtmlMode::Interactive),
            Self::Html { mode, .. } => Some(*mode),
        }
    }
}

struct Lease {
    content: DisplayContent,
    source: Weak<AppBroker>,
    generation: Uuid,
    source_generation: Option<Uuid>,
    _document: DisplayServer,
    input: AppViewInput,
    cancellation: CancellationToken,
}
struct ActiveLease {
    id: Uuid,
    lease: Lease,
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
    active_lease: Mutex<Option<ActiveLease>>,
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
        if let Ok(mut active) = self.store.active_lease.lock() {
            if active
                .as_ref()
                .is_some_and(|active| active.lease.source_generation == Some(self.generation))
            {
                let active = active.take().expect("checked source generation");
                active.lease.cancellation.cancel();
            }
        }
    }
}
#[derive(Serialize)]
pub struct McpServerToolCatalog {
    server: McpAppServer,
    tools: Vec<String>,
}

#[derive(Serialize)]
pub struct OpenedApp {
    pub id: Uuid,
    pub artifact_id: Option<Uuid>,
    pub document_mode: Option<usecase::session_document::HtmlMode>,
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
        let sandbox = adapter_mcp_server::apps::resource_sandbox(&self.resource)?;
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
        if let Some(active) = self
            .active_lease
            .lock()
            .map_err(|_| "MCP lease unavailable")?
            .take()
        {
            active.lease.cancellation.cancel();
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
        let mut active = self
            .active_lease
            .lock()
            .map_err(|_| "MCP lease unavailable")?;
        let runtime = state
            .runtime
            .read()
            .map_err(|_| "Application state unavailable")?;
        let operation_id = runtime.lens.operation_id;
        if active.as_ref().is_some_and(|active| {
            active
                .lease
                .content
                .artifact()
                .is_some_and(|artifact| Some(artifact.descriptor.operation_id) != operation_id)
        }) {
            let active = active.take().expect("checked operation");
            active.lease.cancellation.cancel();
        } else if let Some(active) = active.as_mut() {
            if !self.lease_live(&runtime, &active.lease) {
                // Pause retires RPC authority, but preserves the read-only display.
                active.lease.cancellation.cancel();
            }
        }
        Ok(())
    }
    fn lease_live(&self, runtime: &AppSnapshot, lease: &Lease) -> bool {
        self.source
            .lock()
            .ok()
            .is_some_and(|source| self.lease_live_with_source(runtime, lease, source.as_ref()))
    }

    fn lease_live_with_source(
        &self,
        runtime: &AppSnapshot,
        lease: &Lease,
        source: Option<&SourceAuthority>,
    ) -> bool {
        let Some(artifact) = lease.content.artifact() else {
            return false;
        };
        if self.ensure_open().is_err()
            || lease.cancellation.is_cancelled()
            || !operation_eligible(runtime, &artifact.descriptor)
        {
            return false;
        }
        source.is_some_and(|source| {
            authority_facts(runtime).source_matches(
                SourceAuthorityFacts {
                    operation_id: source.operation_id,
                    session_id: &source.session_id,
                    generation: source.generation,
                    config: &source.config,
                },
                &artifact.descriptor,
                lease.source_generation,
            ) && Weak::ptr_eq(&source.broker, &lease.source)
                && lease.source.upgrade().is_some()
        })
    }

    fn with_live_source<T>(
        &self,
        state: &AppState,
        lease: &Lease,
        admit: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        // Retirement and synchronous admission share one source-authority boundary.
        // All runtime→source readers also own active_lease, as this caller does.
        let source = self.source.lock().map_err(|_| "MCP source unavailable")?;
        if !self.lease_live_with_source(&state.snapshot()?, lease, source.as_ref()) {
            return Err("App session authority expired".into());
        }
        let result = admit();
        drop(source);
        result
    }
    fn commit_display(
        &self,
        state: &AppState,
        ticket: u64,
        id: Uuid,
        lease: Lease,
    ) -> Result<bool, String> {
        let mut active = self
            .active_lease
            .lock()
            .map_err(|_| "MCP lease state unavailable")?;
        self.ensure_open()?;
        if self.open_epoch.load(std::sync::atomic::Ordering::Acquire) != ticket {
            return Err("App display request was superseded".into());
        }
        let install = |active: &mut Option<ActiveLease>, lease: Lease, live: bool| {
            if let Some(previous) = active.take() {
                previous.lease.cancellation.cancel();
            }
            *active = Some(ActiveLease { id, lease });
            Ok(live)
        };
        match &lease.content {
            DisplayContent::Html {
                source:
                    HtmlPresentationSource::History {
                        generation,
                        entry_id,
                        revision,
                        block_index,
                    },
                mode,
            } => {
                let (generation, entry_id, revision, block_index, expected_mode) = (
                    *generation,
                    entry_id.clone(),
                    *revision,
                    *block_index,
                    *mode,
                );
                state.session_view.with_html_document(
                    generation,
                    &entry_id,
                    revision,
                    block_index,
                    |_, mode, _| {
                        if mode != expected_mode {
                            return Err("History HTML permission changed".into());
                        }
                        install(&mut active, lease, false)
                    },
                )
            }
            _ => {
                let latest = state
                    .runtime
                    .read()
                    .map_err(|_| "Application state unavailable")?;
                match &lease.content {
                    DisplayContent::Mcp(artifact) => {
                        if !authority_facts(&latest).display_retained(
                            &artifact.descriptor,
                            latest.lens.mcp_apps.iter().chain(
                                latest
                                    .lens
                                    .response_history
                                    .responses
                                    .iter()
                                    .flat_map(|response| response.mcp_apps.iter()),
                            ),
                        ) {
                            return Err("App operation expired before display opened".into());
                        }
                    }
                    DisplayContent::Html {
                        source:
                            HtmlPresentationSource::Live {
                                output_ref,
                                block_index,
                            },
                        ..
                    } => {
                        let block = crate::commands::response_block(
                            &latest.lens,
                            output_ref.operation_id,
                            output_ref.representation_id,
                            *block_index,
                        )?;
                        if !matches!(block, crate::model::LensOutputBlock::Html { .. }) {
                            return Err("Selected response block is not HTML".into());
                        }
                    }
                    _ => unreachable!("history branch handled above"),
                }
                let live = self.lease_live(&latest, &lease);
                install(&mut active, lease, live)
            }
        }
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

    #[cfg(test)]
    pub(crate) fn artifact_path_for_test(&self, id: Uuid) -> Result<std::path::PathBuf, String> {
        self.artifact_path(id)
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
        let leases=self.active_lease.lock().map(|active| active.iter().map(|active| { let l = &active.lease; json!({"id":active.id,"artifact_id":l.content.artifact().map(|artifact| artifact.descriptor.id),"generation":l.generation,"source_live":l.source.upgrade().is_some(),"authority_active":!l.cancellation.is_cancelled()}) }).collect::<Vec<_>>()).unwrap_or_default();
        json!({"source_live":source_live,"leases":leases})
    }
    fn tool_catalogs(&self, runtime: &AppSnapshot) -> Result<Vec<McpServerToolCatalog>, String> {
        self.ensure_open()?;
        let source = self
            .source
            .lock()
            .map_err(|_| "MCP source state unavailable")?;
        let Some(source) = source.as_ref().filter(|source| {
            runtime.lens.operation_id == Some(source.operation_id)
                && source.config.same_active_session_config(&runtime.config)
        }) else {
            return Ok(Vec::new());
        };
        let Some(broker) = source.broker.upgrade() else {
            return Ok(Vec::new());
        };
        Ok(source
            .config
            .mcp_apps_servers
            .iter()
            .filter_map(|server| {
                broker
                    .model_tool_names(&server.id.to_string())
                    .map(|tools| McpServerToolCatalog {
                        server: server.clone(),
                        tools,
                    })
            })
            .collect())
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
            // A successful tool effect may have no loadable App resource. Preserve
            // the upstream result without publishing an unusable descriptor.
            if validate_resource(&artifact).is_err() {
                continue;
            }
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
    adapter_mcp_server::apps::resource_csp(&artifact.resource)?;
    if artifact.resource.html.is_empty() {
        return Err(
            "App resource could not be retained; the original tool result is preserved".into(),
        );
    }
    Ok(())
}
fn authority_facts(runtime: &AppSnapshot) -> AppAuthorityFacts<'_> {
    AppAuthorityFacts {
        operation_id: runtime.lens.operation_id,
        lifecycle: runtime.lens.live.as_ref().map(|live| live.lifecycle),
        config: &runtime.config,
    }
}
fn operation_eligible(runtime: &AppSnapshot, descriptor: &McpAppDescriptor) -> bool {
    authority_facts(runtime).operation_eligible(descriptor)
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

fn html_source(
    state: &AppState,
    source: &HtmlPresentationSource,
) -> Result<(String, usecase::session_document::HtmlMode, Value), String> {
    match source {
        HtmlPresentationSource::Live {
            output_ref,
            block_index,
        } => {
            let runtime = state
                .runtime
                .read()
                .map_err(|_| "Application state unavailable")?;
            let block = crate::commands::response_block(
                &runtime.lens,
                output_ref.operation_id,
                output_ref.representation_id,
                *block_index,
            )?;
            match block {
                crate::model::LensOutputBlock::Html { text, .. } => {
                    Ok((text, usecase::session_document::HtmlMode::Static, json!({})))
                }
                _ => Err("Selected response block is not HTML".into()),
            }
        }
        HtmlPresentationSource::History {
            generation,
            entry_id,
            revision,
            block_index,
        } => state.session_view.with_html_document(
            *generation,
            entry_id,
            *revision,
            *block_index,
            |html, mode, csp| Ok((html, mode, csp)),
        ),
    }
}
fn document_policy(
    mode: usecase::session_document::HtmlMode,
    csp: Value,
) -> adapter_mcp_server::apps::DocumentPolicy {
    match mode {
        usecase::session_document::HtmlMode::Interactive => {
            adapter_mcp_server::apps::DocumentPolicy::Interactive { csp }
        }
        usecase::session_document::HtmlMode::Static => {
            use base64::Engine;
            use sha2::Digest;
            let helper = include_str!(
                "../../../../packages/adapter-mcp-apps-view/src/assets/rich-html-links.js"
            );
            adapter_mcp_server::apps::DocumentPolicy::Static {
                trusted_script_sha256: base64::engine::general_purpose::STANDARD
                    .encode(sha2::Sha256::digest(helper.as_bytes())),
            }
        }
    }
}
#[tauri::command]
pub async fn open_html_presentation<R: tauri::Runtime>(
    app: AppHandle<R>,
    window: tauri::WebviewWindow<R>,
    source: HtmlPresentationSource,
    host_origin: String,
) -> Result<OpenedApp, String> {
    ensure_host(&app, &window)?;
    if !host_origin_matches(
        &window.url().map_err(|_| "Unable to inspect Host URL")?,
        &host_origin,
    ) {
        return Err("HTML Host origin mismatch".into());
    }
    let state = app.state::<AppState>();
    let ticket = state
        .mcp_apps
        .open_epoch
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
        + 1;
    let (html, mode, csp) = html_source(&state, &source)?;
    if html.len() > adapter_mcp_server::MAX_HTML_BYTES {
        return Err("HTML document exceeds size limit".into());
    }
    let resource = AppResource {
        html: include_str!(
            "../../../../packages/adapter-mcp-apps-view/src/assets/rich-html-app.html"
        )
        .into(),
        meta: json!({"ui":{"csp":csp}}),
    };
    let display = DisplayServer::start(
        resource.clone(),
        Some(html.clone()),
        host_origin,
        include_str!("../../../../packages/adapter-mcp-apps-host/src/assets/sandbox-proxy.html")
            .into(),
        include_str!("../../../../packages/adapter-mcp-apps-host/src/assets/sandbox-proxy.js")
            .into(),
        document_policy(mode, csp),
    )
    .await
    .map_err(|_| "Unable to start HTML sandbox")?;
    let id = Uuid::new_v4();
    let mut opened = OpenedApp {
        id,
        artifact_id: None,
        document_mode: Some(mode),
        proxy_url: display.proxy_url.clone(),
        proxy_origin: display.origin.clone(),
        resource,
        // Display-only shell input; this is not a saved MCP transaction.
        input: json!({"html": html}),
        result: json!({"content": []}),
        host_capabilities: json!({}),
        live: false,
        document_url: display.document_url.clone(),
    };
    state.mcp_apps.commit_display(
        &state,
        ticket,
        id,
        Lease {
            content: DisplayContent::Html { source, mode },
            source: Weak::new(),
            generation: Uuid::new_v4(),
            source_generation: None,
            _document: display,
            input: AppViewInput::default(),
            cancellation: CancellationToken::new(),
        },
    )?;
    opened.set_live(false)?;
    Ok(opened)
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
    let document = if artifact.artifact.server_id == adapter_mcp_server::apps::FALLBACK_SERVER {
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
        include_str!("../../../../packages/adapter-mcp-apps-host/src/assets/sandbox-proxy.html")
            .into(),
        include_str!("../../../../packages/adapter-mcp-apps-host/src/assets/sandbox-proxy.js")
            .into(),
        adapter_mcp_server::apps::DocumentPolicy::Interactive {
            csp: artifact
                .artifact
                .resource
                .meta
                .pointer("/ui/csp")
                .cloned()
                .unwrap_or_else(|| json!({})),
        },
    )
    .await
    .map_err(|_| "Unable to start App sandbox")?;
    let result = artifact.artifact.result.clone();
    // A Host-owned display location is separate from the immutable source tool result.
    // The SDK gets the original result; the bundled shell reads this Host-context value.
    let mut response = OpenedApp {
        id,
        artifact_id: Some(artifact_id),
        document_mode: display
            .document_url
            .as_ref()
            .map(|_| usecase::session_document::HtmlMode::Interactive),
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
            content: DisplayContent::Mcp(Box::new(artifact)),
            source,
            generation,
            source_generation,
            _document: display,
            input: AppViewInput::default(),
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
        .active_lease
        .lock()
        .map_err(|_| "MCP lease state unavailable")?
        .take_if(|active| active.id == lease_id)
    {
        lease.lease.cancellation.cancel();
    }
    Ok(())
}

fn ensure_display(state: &AppState, lease: &Lease) -> Result<(), String> {
    state.mcp_apps.ensure_open()?;
    match &lease.content {
        DisplayContent::Mcp(artifact) => {
            let snapshot = state.snapshot()?;
            if !authority_facts(&snapshot).display_retained(
                &artifact.descriptor,
                snapshot.lens.mcp_apps.iter().chain(
                    snapshot
                        .lens
                        .response_history
                        .responses
                        .iter()
                        .flat_map(|response| response.mcp_apps.iter()),
                ),
            ) {
                return Err("App display expired".into());
            }
        }
        DisplayContent::Html { source, .. } => {
            html_source(state, source)?;
        }
    }
    Ok(())
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
    let active = state
        .mcp_apps
        .active_lease
        .lock()
        .map_err(|_| "MCP lease unavailable")?;
    let lease = &active
        .as_ref()
        .filter(|active| active.id == lease_id)
        .ok_or("App lease expired")?
        .lease;
    ensure_display(&state, lease)?;
    if lease.content.document_mode().is_none() {
        return Err("Only HTML documents can be prepared".into());
    }
    lease._document.prepare_document(document)
}

fn open_link_with(
    state: &AppState,
    lease: &Lease,
    params: &Value,
    open: impl FnOnce(String) -> Result<(), String>,
) -> Result<(), String> {
    ensure_display(state, lease)?;
    let destination = usecase::mcp_apps::link_destination(params)?;
    open(destination)
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
        let mut active = state
            .mcp_apps
            .active_lease
            .lock()
            .map_err(|_| "MCP lease state unavailable")?;
        let lease = &mut active
            .as_mut()
            .filter(|active| active.id == lease_id)
            .ok_or("App lease expired")?
            .lease;
        if method == "ui/open-link" {
            open_link_with(&state, lease, &params, |destination| {
                app.opener()
                    .open_url(destination, None::<&str>)
                    .map_err(|_| "Unable to open HTML link in default browser".into())
            })?;
            return Ok(AppRequestResult { result: json!({}) });
        }
        if !state.mcp_apps.lease_live(&state.snapshot()?, lease) {
            return Err("App session authority expired".into());
        }
        match method {
            "ui/update-model-context" => {
                lease.input.replace_context(params)?;
                return Ok(AppRequestResult { result: json!({}) });
            }
            "ui/message" => {
                let text = usecase::mcp_apps::text_content(&params)?;
                let artifact = lease
                    .content
                    .artifact()
                    .ok_or("App source authority unavailable")?;
                let prompt = app_message_prompt(&artifact.descriptor, &text, lease.input.context());
                state.mcp_apps.with_live_source(&state, lease, || {
                    state
                        .agent_control
                        .submit_app_message(&app, &artifact.descriptor, prompt)
                })?;
                return Ok(AppRequestResult { result: json!({}) });
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
            lease
                .content
                .artifact()
                .ok_or("App source authority unavailable")?
                .artifact
                .clone(),
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
    let active = state
        .mcp_apps
        .active_lease
        .lock()
        .map_err(|_| "MCP lease state unavailable")?;
    let runtime = state
        .runtime
        .read()
        .map_err(|_| "Application state unavailable")?;
    if active
        .as_ref()
        .filter(|active| active.id == lease_id)
        .is_none_or(|active| {
            active.lease.generation != generation
                || !state.mcp_apps.lease_live(&runtime, &active.lease)
        })
        || cancellation.is_cancelled()
    {
        return Err("App lease expired before the tool result returned".into());
    }
    Ok(AppRequestResult { result })
}

#[tauri::command]
pub fn get_mcp_server_tool_catalogs<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<McpServerToolCatalog>, String> {
    let state = app.state::<AppState>();
    // Do not retain runtime authority while acquiring the independent source mutex.
    let runtime = state.snapshot()?;
    state.mcp_apps.tool_catalogs(&runtime)
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
    async fn source_retirement_cannot_overtake_synchronous_message_admission() {
        use std::sync::{mpsc, TryLockError};
        let state = crate::test_support::state();
        let operation = Uuid::new_v4();
        let descriptor = descriptor(operation);
        state.runtime.write().unwrap().lens.operation_id = Some(operation);
        let store = Arc::clone(&state.mcp_apps);
        let broker = Arc::new(
            AppBroker::start(vec![], "<p>shell</p>".into())
                .await
                .unwrap(),
        );
        let lifetime = store
            .install(
                operation,
                descriptor.session_id.clone(),
                &broker,
                &state.config().unwrap(),
            )
            .unwrap();
        let generation = lifetime.generation;
        let display = lease(
            descriptor.clone(),
            Arc::downgrade(&broker),
            Some(generation),
        )
        .await;
        let cancellation = display.cancellation.clone();
        *store.active_lease.lock().unwrap() = Some(ActiveLease {
            id: Uuid::new_v4(),
            lease: display,
        });
        let (start, started) = mpsc::channel();
        let (blocked, observed) = mpsc::channel();
        let (retired, retirement) = mpsc::channel();
        std::thread::scope(|scope| {
            let retiring_store = Arc::clone(&store);
            let retirement_thread = scope.spawn(move || {
                started.recv().unwrap();
                let protected = matches!(
                    retiring_store.source.try_lock(),
                    Err(TryLockError::WouldBlock)
                );
                blocked.send(protected).unwrap();
                drop(lifetime);
                retired.send(()).unwrap();
            });
            {
                // Production admission already owns this display gate.
                let active = store.active_lease.lock().unwrap();
                let lease = &active.as_ref().unwrap().lease;
                store
                    .with_live_source(&state, lease, || {
                        start.send(()).unwrap();
                        assert!(observed.recv().unwrap());
                        assert!(matches!(
                            retirement.try_recv(),
                            Err(mpsc::TryRecvError::Empty)
                        ));
                        assert!(!cancellation.is_cancelled());
                        Ok(())
                    })
                    .unwrap();
            }
            retirement.recv().unwrap();
            retirement_thread.join().unwrap();
        });
        assert!(cancellation.is_cancelled());
        assert!(store.source.lock().unwrap().is_none());
        assert!(store.active_lease.lock().unwrap().is_none());
        let stale = lease(descriptor, Arc::downgrade(&broker), Some(generation)).await;
        assert!(store
            .with_live_source::<()>(&state, &stale, || {
                panic!("retired source must not admit a late message")
            })
            .is_err());
        broker.close().await;
    }
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
        assert!(store.active_lease.lock().unwrap().is_none());
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
    fn opened_app_serializes_only_implemented_live_content_modalities() {
        let mut opened = OpenedApp {
            id: Uuid::new_v4(),
            artifact_id: Some(Uuid::new_v4()),
            document_mode: None,
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
        let policy = adapter_mcp_server::apps::resource_csp(&opened.resource).unwrap();
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
            content: DisplayContent::Mcp(Box::new(RetainedArtifact {
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
            })),
            source,
            generation: Uuid::new_v4(),
            source_generation,
            _document: DisplayServer::start(
                resource,
                None,
                "tauri://localhost".into(),
                "__SETTINGS_JSON__".into(),
                String::new(),
                adapter_mcp_server::apps::DocumentPolicy::Interactive { csp: json!({}) },
            )
            .await
            .unwrap(),
            input: AppViewInput::default(),
            cancellation: CancellationToken::new(),
        }
    }
    #[test]
    fn static_document_policy_hashes_exact_trusted_link_helper_bytes() {
        use base64::Engine;
        use sha2::Digest;
        let policy = document_policy(usecase::session_document::HtmlMode::Static, json!({}));
        let adapter_mcp_server::apps::DocumentPolicy::Static {
            trusted_script_sha256,
        } = policy
        else {
            panic!("static history must not enable authored JavaScript");
        };
        assert_eq!(
            trusted_script_sha256,
            base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(include_bytes!(
                "../../../../packages/adapter-mcp-apps-view/src/assets/rich-html-links.js"
            )))
        );
        assert!(matches!(
            document_policy(usecase::session_document::HtmlMode::Interactive, json!({})),
            adapter_mcp_server::apps::DocumentPolicy::Interactive { .. }
        ));
    }

    #[tokio::test]
    async fn html_presentation_command_returns_exact_prepare_input_without_mcp_artifact_authority()
    {
        use usecase::session_document::{DocumentEntry, HtmlMode, SessionDocument};
        let state = crate::test_support::state();
        let html = "<p>saved</p><script>localTabs()</script>";
        let mut document = SessionDocument::default();
        document.entries.push(DocumentEntry::Tool {
            id: "render".into(),
            title: "render_html".into(),
            status: agent_client_protocol::schema::v1::ToolCallStatus::Completed,
            blocks: vec![],
            accepted_html: Some(html.into()),
            accepted_html_mode: HtmlMode::Interactive,
            accepted_html_csp: Some(json!({"resourceDomains":["https://cdn.example"],"connectDomains":["https://api.example"]})),
        });
        state
            .session_view
            .install_validation_document(document)
            .unwrap();
        let source = HtmlPresentationSource::History {
            generation: state.session_view.configuration_authority().unwrap().0,
            entry_id: "render".into(),
            revision: state.session_view.view().unwrap().revision,
            block_index: 0,
        };
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(crate::product_context())
            .unwrap();
        let overlay = tauri::WebviewWindowBuilder::new(
            &app,
            crate::ui::LENS_WINDOW_LABEL,
            Default::default(),
        )
        .build()
        .unwrap();
        let origin = overlay.url().unwrap().origin().ascii_serialization();
        let opened = open_html_presentation(app.handle().clone(), overlay.clone(), source, origin)
            .await
            .unwrap();
        let serialized = serde_json::to_value(&opened).unwrap();
        assert!(serialized["artifact_id"].is_null());
        assert_eq!(serialized["document_mode"], "interactive");
        assert_eq!(serialized["input"]["html"], html);
        assert_eq!(serialized["result"], json!({"content": []}));
        assert_eq!(serialized["live"], false);
        assert!(serialized["host_capabilities"].get("message").is_none());
        assert!(serialized["host_capabilities"].get("serverTools").is_none());
        assert_eq!(
            serialized["resource"]["meta"]["ui"]["csp"],
            json!({"resourceDomains":["https://cdn.example"],"connectDomains":["https://api.example"]})
        );
        assert_eq!(
            serialized["host_capabilities"]["sandbox"]["csp"],
            serialized["resource"]["meta"]["ui"]["csp"]
        );
        let document_response = reqwest::get(opened.document_url.unwrap()).await.unwrap();
        let csp_header = document_response
            .headers()
            .get(reqwest::header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp_header.contains("script-src 'unsafe-inline' https://cdn.example"));
        assert!(csp_header.contains("connect-src https://api.example"));
        assert!(csp_header.contains("frame-src 'none'"));
        assert_eq!(document_response.text().await.unwrap(), html);
        prepare_mcp_app_document(
            app.handle().clone(),
            overlay,
            opened.id,
            "<p>prepared</p>".into(),
        )
        .unwrap();
        assert!(mcp_app_request(app.handle().clone(), opened.id, json!({"method": "ui/message", "params": {"content": [{"type": "text", "text": "forbidden"}]}})).await.is_err());
        assert!(mcp_app_request(
            app.handle().clone(),
            opened.id,
            json!({"method": "tools/list"})
        )
        .await
        .is_err());
        close_mcp_app(app.handle().clone(), opened.id).unwrap();
    }

    #[tokio::test]
    async fn archived_html_display_has_only_local_and_link_authority_and_rejects_replacement() {
        use usecase::session_document::{DocumentEntry, HtmlMode, SessionDocument};
        let state = crate::test_support::state();
        let mut document = SessionDocument::default();
        document.entries.push(DocumentEntry::Tool {
            id: "render".into(),
            title: "render_html".into(),
            status: agent_client_protocol::schema::v1::ToolCallStatus::Completed,
            blocks: vec![],
            accepted_html: Some("<button>Details</button><script>localTabs()</script>".into()),
            accepted_html_mode: HtmlMode::Interactive,
            accepted_html_csp: Some(
                json!({"resourceDomains":["https://cdn.example"],"connectDomains":[]}),
            ),
        });
        state
            .session_view
            .install_validation_document(document.clone())
            .unwrap();
        let view = state.session_view.view().unwrap();
        let source = HtmlPresentationSource::History {
            generation: state.session_view.configuration_authority().unwrap().0,
            entry_id: "render".into(),
            revision: view.revision,
            block_index: 0,
        };
        assert_eq!(
            html_source(&state, &source).unwrap().1,
            HtmlMode::Interactive
        );
        assert_eq!(
            html_source(&state, &source).unwrap().2,
            json!({"resourceDomains":["https://cdn.example"],"connectDomains":[]})
        );
        let mut stale_revision = source.clone();
        if let HtmlPresentationSource::History { revision, .. } = &mut stale_revision {
            *revision += 1;
        }
        assert!(html_source(&state, &stale_revision).is_err());
        let display = DisplayServer::start(
            AppResource {
                html: "<p>shell</p>".into(),
                meta: json!({}),
            },
            Some("<p>document</p>".into()),
            "tauri://localhost".into(),
            "__SETTINGS_JSON__".into(),
            String::new(),
            document_policy(HtmlMode::Interactive, json!({})),
        )
        .await
        .unwrap();
        let endpoint = display.proxy_url.clone();
        let id = Uuid::new_v4();
        let lease = Lease {
            content: DisplayContent::Html {
                source: source.clone(),
                mode: HtmlMode::Interactive,
            },
            source: Weak::new(),
            source_generation: None,
            generation: Uuid::new_v4(),
            _document: display,
            input: AppViewInput::default(),
            cancellation: CancellationToken::new(),
        };
        assert!(!state.mcp_apps.commit_display(&state, 0, id, lease).unwrap());
        assert!(state.runtime.read().unwrap().lens.operation_id.is_none());
        {
            let active = state.mcp_apps.active_lease.lock().unwrap();
            let lease = &active.as_ref().unwrap().lease;
            assert!(!state.mcp_apps.lease_live(&state.snapshot().unwrap(), lease));
            let mut opened = None;
            open_link_with(
                &state,
                lease,
                &json!({"url":"https://example.com/?from=history#details"}),
                |destination| {
                    opened = Some(destination);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(
                opened.as_deref(),
                Some("https://example.com/?from=history#details")
            );
        }
        // A different archived selection has a new generation even when its entry/body are equal.
        state
            .session_view
            .install_validation_document(document)
            .unwrap();
        {
            let active = state.mcp_apps.active_lease.lock().unwrap();
            let lease = &active.as_ref().unwrap().lease;
            assert!(ensure_display(&state, lease).is_err());
            assert!(open_link_with(
                &state,
                lease,
                &json!({"url":"https://example.com/"}),
                |_| panic!("expired archive must not open links")
            )
            .is_err());
        }
        state.session_view.clear().unwrap();
        assert!(html_source(&state, &source).is_err());
        {
            let active = state.mcp_apps.active_lease.lock().unwrap();
            assert!(ensure_display(&state, &active.as_ref().unwrap().lease).is_err());
        }
        let old = state.mcp_apps.active_lease.lock().unwrap().take().unwrap();
        old.lease.cancellation.cancel();
        drop(old);
        tokio::task::yield_now().await;
        assert!(reqwest::Client::new().get(endpoint).send().await.is_err());
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
        *store.active_lease.lock().unwrap() = Some(ActiveLease {
            id: new_id,
            lease: new_lease,
        });
        drop(old);
        assert!(store
            .active_lease
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|active| active.id == new_id));
        assert!(!cancellation.is_cancelled());
        assert_eq!(
            store.source.lock().unwrap().as_ref().unwrap().generation,
            newer.generation
        );
        drop(newer);
        assert!(store.active_lease.lock().unwrap().is_none());
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
        let new_endpoint = new._document.proxy_url.clone();
        let new_id = Uuid::new_v4();
        let new_cancel = new.cancellation.clone();
        assert!(!store
            .commit_display(&state, new_ticket, new_id, new)
            .unwrap());
        assert!(store
            .commit_display(&state, old_ticket, Uuid::new_v4(), old)
            .is_err());
        assert!(store
            .active_lease
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|active| active.id == new_id));
        assert!(!new_cancel.is_cancelled());
        tokio::task::yield_now().await;
        assert!(reqwest::Client::new()
            .get(old_endpoint)
            .send()
            .await
            .is_err());
        let replacement = descriptor(operation);
        state
            .runtime
            .write()
            .unwrap()
            .lens
            .mcp_apps
            .push(replacement.clone());
        let replacement = lease(replacement, Weak::new(), None).await;
        let replacement_id = Uuid::new_v4();
        let replacement_ticket = store
            .open_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        store
            .commit_display(&state, replacement_ticket, replacement_id, replacement)
            .unwrap();
        assert!(new_cancel.is_cancelled());
        assert!(store
            .active_lease
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|active| active.id == replacement_id));
        tokio::task::yield_now().await;
        assert!(reqwest::Client::new()
            .get(new_endpoint)
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
        if let DisplayContent::Mcp(artifact) = &mut display.content {
            artifact.artifact.server_id = adapter_mcp_server::apps::FALLBACK_SERVER.into();
        }
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
        // A delayed close for another display must not remove the active lease.
        close_mcp_app(handle.clone(), Uuid::new_v4()).unwrap();
        let state = app.state::<AppState>();
        {
            let active = state.mcp_apps.active_lease.lock().unwrap();
            let lease = &active.as_ref().unwrap().lease;
            let mut opened = Vec::new();
            for url in ["https://example.com/first", "https://example.com/second"] {
                open_link_with(&state, lease, &json!({"url":url}), |destination| {
                    opened.push(destination);
                    Ok(())
                })
                .unwrap();
            }
            assert_eq!(
                opened,
                ["https://example.com/first", "https://example.com/second"]
            );
            assert!(open_link_with(
                &state,
                lease,
                &json!({"url":"file:///private"}),
                |_| panic!("must not open")
            )
            .is_err());
        }
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
            state
                .mcp_apps
                .is_live(&display.content.artifact().unwrap().descriptor),
            "old source is still registered while its captured configuration is retired"
        );
        drop(lifetime);
    }
}
