use crate::{
    app_state::{publish_agent_runtime, update_agent_runtime, AppState},
    model::{AgentKind, AgentRuntimeStage, AgentRuntimeState},
};
use flate2::read::GzDecoder;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::{
    fs::{self, File},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::{io::AsyncWriteExt, process::Command};
use uuid::Uuid;

const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const REGISTRY_SCHEMA_VERSION: &str = "1.0.0";
const NODE_VERSION: &str = env!("LENS_NODE_VERSION");
const NODE_TARGET: &str = env!("LENS_NODE_TARGET");
const NODE_ARCHIVE_NAME: &str = env!("LENS_NODE_ARCHIVE_NAME");
const NODE_ARCHIVE_ROOT: &str = env!("LENS_NODE_ARCHIVE_ROOT");
const NODE_ARCHIVE_URL: &str = env!("LENS_NODE_ARCHIVE_URL");
const NODE_ARCHIVE_SHA256: &str = env!("LENS_NODE_ARCHIVE_SHA256");
const NODE_ARCHIVE_MAX_BYTES: u64 = 64 * 1024 * 1024;
const PNPM_VERSION: &str = "11.22.0";
const PNPM_ARCHIVE_NAME: &str = "pnpm-11.22.0.tgz";
const PNPM_ARCHIVE_ROOT: &str = "package";
const PNPM_ARCHIVE_URL: &str = "https://npm.flatt.tech/pnpm/-/pnpm-11.22.0.tgz";
const PNPM_ARCHIVE_SHA512: &str = "1ff870c4c6133dfd88fb2afc46dd13d47f09c9794b438c6fdb47ca98caf3bc16381ee0be93a091b8e3824cf01f889f46d7d9e20910fb0be1ab0fb5baa80dd621";
const PNPM_CLI_SHA256: &str = "ff3224d46b47fbb24a7e9fe15fededef7e00892d07d4e376b6762d4899906bfd";
const PNPM_DIST_SHA256: &str = "a8533087155540515892e6f022ba5c673bb2e62fcbcc124b36ba2e8938ccc3da";
const PNPM_ARCHIVE_MAX_BYTES: u64 = 16 * 1024 * 1024;
const TAKUMI_GUARD_REGISTRY: &str = "https://npm.flatt.tech/";
const REGISTRY_MAX_BYTES: usize = 2 * 1024 * 1024;
const AGENT_INSTALL_RECORD_VERSION: u32 = 6;
const NODE_INSTALL_RECORD_VERSION: u32 = 2;
const PNPM_INSTALL_RECORD_VERSION: u32 = 1;
const NODE_TEAM_ID: &str = "HX7739G8FX";
const NODE_SIGNING_IDENTIFIER: &str = "node";
const CLAUDE_TEAM_ID: &str = "Q6L2SF6YDW";
const OPENAI_TEAM_ID: &str = "2DC432GLL2";

const AGENT_WORKSPACE: &[u8] = include_bytes!("../agent-runtime/pnpm-workspace.yaml");

#[derive(Debug, Clone)]
pub struct ResolvedAgentRuntime {
    pub kind: AgentKind,
    pub adapter_name: &'static str,
    pub adapter_version: String,
    pub safe_mode_id: &'static str,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub(crate) installation: Option<std::sync::Arc<RuntimeInstallation>>,
}

/// A lease keeps immutable files available while any Agent process references them.
#[derive(Debug)]
pub(crate) struct RuntimeInstallation {
    root: PathBuf,
    kind: AgentKind,
    id: String,
}
impl Drop for RuntimeInstallation {
    fn drop(&mut self) {
        if let Ok(_guard) = selector_mutex().lock() {
            let _ = prune_installations(&self.root, self.kind);
        }
    }
}
type LeaseMap = std::collections::HashMap<PathBuf, std::sync::Weak<RuntimeInstallation>>;
fn leases() -> &'static std::sync::Mutex<LeaseMap> {
    static VALUE: std::sync::OnceLock<std::sync::Mutex<LeaseMap>> = std::sync::OnceLock::new();
    VALUE.get_or_init(Default::default)
}
fn selector_mutex() -> &'static std::sync::Mutex<()> {
    static VALUE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    &VALUE
}
#[derive(Clone, Copy)]
struct Provider {
    kind: AgentKind,
    registry_id: &'static str,
    adapter_name: &'static str,
    safe_mode_id: &'static str,
    bin_name: &'static str,
}
fn provider(kind: AgentKind) -> Provider {
    match kind {
        AgentKind::Claude => Provider {
            kind,
            registry_id: "claude-acp",
            adapter_name: "@agentclientprotocol/claude-agent-acp",
            safe_mode_id: "plan",
            bin_name: "claude-agent-acp",
        },
        AgentKind::Codex => Provider {
            kind,
            registry_id: "codex-acp",
            adapter_name: "@agentclientprotocol/codex-acp",
            safe_mode_id: "read-only",
            bin_name: "codex-acp",
        },
    }
}
#[derive(Debug, Deserialize)]
struct RegistryIndex {
    version: String,
    agents: Vec<RegistryAgent>,
}
#[derive(Debug, Deserialize)]
struct RegistryAgent {
    id: String,
    version: String,
    distribution: RegistryDistribution,
}
#[derive(Debug, Deserialize)]
struct RegistryDistribution {
    #[serde(default)]
    npx: Option<RegistryNpxDistribution>,
}
#[derive(Debug, Deserialize)]
struct RegistryNpxDistribution {
    package: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstallRecord {
    schema_version: u32,
    registry_id: String,
    adapter_name: String,
    adapter_version: String,
    node_version: String,
    node_archive_sha256: String,
    pnpm_version: String,
    pnpm_archive_sha512: String,
    pnpm_lock_sha256: String,
    pnpm_workspace_sha256: String,
    #[serde(default)]
    package_json_sha256: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeInstallRecord {
    schema_version: u32,
    node_version: String,
    node_target: String,
    archive_sha256: String,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct PnpmInstallRecord {
    schema_version: u32,
    pnpm_version: String,
    registry: String,
    archive_sha512: String,
    cli_sha256: String,
    dist_sha256: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Selector {
    current: Option<String>,
    previous: Option<String>,
    candidate: Option<String>,
    #[serde(default)]
    rejected_version: Option<String>,
}
fn provider_root(root: &Path, kind: AgentKind) -> PathBuf {
    root.join("agents").join(provider(kind).registry_id)
}
fn install_root(root: &Path, kind: AgentKind, id: &str) -> Result<PathBuf, String> {
    if let Some(version) = id.strip_prefix("legacy:") {
        if !valid_version(version) {
            return Err("invalid legacy installation identity".into());
        }
        return Ok(provider_root(root, kind).join(version));
    }
    Uuid::parse_str(id).map_err(|_| "invalid managed installation identity".to_string())?;
    Ok(provider_root(root, kind).join("installs").join(id))
}
fn read_selector(root: &Path, kind: AgentKind) -> Result<Selector, String> {
    let path = provider_root(root, kind).join("selector.json");
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Selector::default())
        }
        Err(error) => return Err(format!("unable to read runtime selector: {error}")),
    };
    let selector: Selector = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid runtime selector: {error}"))?;
    for id in [&selector.current, &selector.previous, &selector.candidate]
        .into_iter()
        .flatten()
    {
        install_root(root, kind, id)?;
    }
    Ok(selector)
}
fn write_selector(root: &Path, kind: AgentKind, selector: &Selector) -> Result<(), String> {
    let dir = provider_root(root, kind);
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!(".selector-{}.json", Uuid::new_v4()));
    let result = (|| {
        let mut file = File::create(&path).map_err(|error| error.to_string())?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(selector).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&path, dir.join("selector.json")).map_err(|error| error.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}
fn prune_installations(root: &Path, kind: AgentKind) -> Result<(), String> {
    let selector = read_selector(root, kind)?;
    // Only a confirmed replacement authorizes retiring pre-migration installations.
    // Authentication, cancellation, and candidate rejection can all drop leases first.
    let retire_legacy = selector
        .current
        .as_deref()
        .is_some_and(|id| Uuid::parse_str(id).is_ok());
    let provider = provider_root(root, kind);
    let mut entries = Vec::new();
    for dir in [&provider, &provider.join("installs")] {
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = if dir == &provider && valid_version(&name) {
                if !retire_legacy {
                    continue;
                }
                format!("legacy:{name}")
            } else if dir != &provider && Uuid::parse_str(&name).is_ok() {
                name
            } else {
                continue;
            };
            entries.push((entry.path(), id));
        }
    }
    let mut map = leases().lock().map_err(|_| "runtime leases unavailable")?;
    map.retain(|_, lease| lease.strong_count() > 0);
    for (path, id) in entries {
        if [&selector.current, &selector.previous, &selector.candidate]
            .into_iter()
            .flatten()
            .any(|keep| keep == &id)
            || map.get(&path).is_some_and(|lease| lease.strong_count() > 0)
        {
            continue;
        }
        fs::remove_dir_all(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}
fn acquire_lease(
    root: &Path,
    kind: AgentKind,
    id: &str,
) -> Result<std::sync::Arc<RuntimeInstallation>, String> {
    let path = install_root(root, kind, id)?;
    let mut map = leases().lock().map_err(|_| "runtime leases unavailable")?;
    if let Some(lease) = map.get(&path).and_then(std::sync::Weak::upgrade) {
        return Ok(lease);
    }
    let lease = std::sync::Arc::new(RuntimeInstallation {
        root: root.to_owned(),
        kind,
        id: id.into(),
    });
    map.insert(path, std::sync::Arc::downgrade(&lease));
    Ok(lease)
}
pub(crate) fn confirm_ready(runtime: &ResolvedAgentRuntime) -> Result<(), String> {
    let Some(install) = &runtime.installation else {
        return Ok(());
    };
    let _guard = selector_mutex()
        .lock()
        .map_err(|_| "runtime selector unavailable")?;
    let mut selector = read_selector(&install.root, install.kind)?;
    if selector.current.as_deref() == Some(&install.id) {
        return Ok(());
    }
    if selector.candidate.as_deref() != Some(&install.id) {
        return Err("runtime candidate was superseded".into());
    }
    selector.previous = selector.current.take();
    selector.current = selector.candidate.take();
    selector.rejected_version = None;
    write_selector(&install.root, install.kind, &selector)?;
    prune_installations(&install.root, install.kind)
}
pub(crate) async fn reject_candidate(
    runtime: &ResolvedAgentRuntime,
) -> Result<Option<ResolvedAgentRuntime>, String> {
    let Some(install) = &runtime.installation else {
        return Ok(None);
    };
    let fallback = {
        let _guard = selector_mutex()
            .lock()
            .map_err(|_| "runtime selector unavailable")?;
        let mut selector = read_selector(&install.root, install.kind)?;
        if selector.candidate.as_deref() != Some(&install.id) {
            return Ok(None);
        }
        selector.candidate = None;
        selector.rejected_version = Some(runtime.adapter_version.clone());
        write_selector(&install.root, install.kind, &selector)?;
        selector.current
    };
    match fallback {
        Some(id) => load_runtime(&install.root, install.kind, &id)
            .await
            .map(Some),
        None => Ok(None),
    }
}

pub async fn resolve<R: tauri::Runtime>(
    app: &AppHandle<R>,
    kind: AgentKind,
) -> Result<ResolvedAgentRuntime, String> {
    resolve_available(app, kind, false).await
}
pub async fn resolve_for_session<R: tauri::Runtime>(
    app: &AppHandle<R>,
    kind: AgentKind,
) -> Result<ResolvedAgentRuntime, String> {
    resolve_available(app, kind, true).await
}
pub async fn resolve_installed<R: tauri::Runtime>(
    app: &AppHandle<R>,
    kind: AgentKind,
) -> Result<Option<ResolvedAgentRuntime>, String> {
    let state = app.state::<AppState>();
    let _guard = state.agent_runtime_install.lock().await;
    let operation = Uuid::new_v4();
    publish_agent_runtime(
        app,
        AgentRuntimeState {
            operation_id: Some(operation),
            agent: Some(kind),
            stage: AgentRuntimeStage::Resolving,
            message: Some("Checking the installed Agent runtime…".into()),
            ..Default::default()
        },
    )?;
    let result = async {
        ensure_supported_target()?;
        let root = runtime_root(app)?;
        migrate_legacy(&root, kind).await?;
        selected_runtime(&root, kind).await
    }
    .await;
    match &result {
        Ok(Some(runtime)) => publish_ready(app, operation, runtime, None)?,
        Ok(None) => {
            update_agent_runtime(app, operation, |state| {
                state.stage = AgentRuntimeStage::NotInstalled;
                state.message = Some(format!(
                    "{} will be downloaded when selected.",
                    display_name(kind)
                ));
                state.error = None;
            })?;
        }
        Err(error) => {
            update_agent_runtime(app, operation, |state| {
                state.stage = AgentRuntimeStage::Failed;
                state.message = None;
                state.error = Some(error.clone());
            })?;
        }
    }
    result
}
async fn selected_runtime(
    root: &Path,
    kind: AgentKind,
) -> Result<Option<ResolvedAgentRuntime>, String> {
    let snapshot = read_selector(root, kind)?;
    for id in [snapshot.candidate, snapshot.current, snapshot.previous]
        .into_iter()
        .flatten()
    {
        match load_runtime(root, kind, &id).await {
            Ok(runtime) => {
                let _guard = selector_mutex()
                    .lock()
                    .map_err(|_| "runtime selector unavailable")?;
                let mut selector = read_selector(root, kind)?;
                if selector.previous.as_deref() == Some(&id) && selector.current.is_none() {
                    selector.current = selector.previous.take();
                    write_selector(root, kind, &selector)?;
                }
                return Ok(Some(runtime));
            }
            Err(_) => {
                let _guard = selector_mutex()
                    .lock()
                    .map_err(|_| "runtime selector unavailable")?;
                let mut selector = read_selector(root, kind)?;
                for reference in [
                    &mut selector.candidate,
                    &mut selector.current,
                    &mut selector.previous,
                ] {
                    if reference.as_deref() == Some(&id) {
                        *reference = None;
                    }
                }
                write_selector(root, kind, &selector)?;
            }
        }
    }
    Ok(None)
}
async fn resolve_available<R: tauri::Runtime>(
    app: &AppHandle<R>,
    kind: AgentKind,
    update: bool,
) -> Result<ResolvedAgentRuntime, String> {
    let state = app.state::<AppState>();
    let _guard = state.agent_runtime_install.lock().await;
    let root = runtime_root(app)?;
    resolve_at(app, kind, update, &root).await
}
async fn resolve_at<R: tauri::Runtime>(
    app: &AppHandle<R>,
    kind: AgentKind,
    update: bool,
    root: &Path,
) -> Result<ResolvedAgentRuntime, String> {
    ensure_supported_target()?;
    migrate_legacy(root, kind).await?;
    let existing = selected_runtime(root, kind).await?;
    let operation_id = Uuid::new_v4();
    publish_agent_runtime(
        app,
        AgentRuntimeState {
            operation_id: Some(operation_id),
            stage: AgentRuntimeStage::Resolving,
            agent: Some(kind),
            version: existing
                .as_ref()
                .map(|runtime| runtime.adapter_version.clone()),
            message: Some("Checking the official ACP Registry…".into()),
            ..Default::default()
        },
    )?;
    if !update || read_selector(root, kind)?.candidate.is_some() {
        if let Some(runtime) = &existing {
            publish_ready(app, operation_id, runtime, None)?;
            return Ok(runtime.clone());
        }
    }
    let result = async {
        let version = fetch_registry_version(kind).await?;
        if let Some(runtime) = &existing {
            if version_order(&runtime.adapter_version, &version) != std::cmp::Ordering::Less {
                return Ok(runtime.clone());
            }
        }
        if read_selector(root, kind)?.rejected_version.as_deref() == Some(&version) {
            return Err(format!(
                "{} {version} previously failed required ACP compatibility checks",
                display_name(kind)
            ));
        }
        let node = ensure_node_runtime(app, root, operation_id).await?;
        let pnpm = ensure_pnpm_runtime(app, root, &node, operation_id).await?;
        install_requested_candidate(
            app,
            root,
            &node,
            &pnpm,
            kind,
            CandidateRequest::Eligible {
                ceiling: &version,
                after: existing
                    .as_ref()
                    .map(|runtime| runtime.adapter_version.as_str()),
            },
            operation_id,
        )
        .await
    }
    .await;
    match result {
        Ok(runtime) => {
            publish_ready(app, operation_id, &runtime, None)?;
            Ok(runtime)
        }
        Err(error) => {
            if let Some(runtime) = existing {
                publish_ready(app, operation_id, &runtime, Some(error))?;
                Ok(runtime)
            } else {
                update_agent_runtime(app, operation_id, |state| {
                    state.stage = AgentRuntimeStage::Failed;
                    state.error = Some(error.clone());
                    state.message = None;
                })?;
                Err(error)
            }
        }
    }
}
fn publish_ready<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation: Uuid,
    runtime: &ResolvedAgentRuntime,
    failure: Option<String>,
) -> Result<(), String> {
    update_agent_runtime(app, operation, |state| {
        state.stage = AgentRuntimeStage::Ready;
        state.version = Some(runtime.adapter_version.clone());
        state.message = Some(match &failure {
            Some(error) => format!(
                "Update failed; continuing with verified {} {}: {error}",
                display_name(runtime.kind),
                runtime.adapter_version
            ),
            None => format!(
                "{} {} is installed and verified.",
                display_name(runtime.kind),
                runtime.adapter_version
            ),
        });
        state.error = failure;
        state.downloaded_bytes = 0;
        state.total_bytes = None;
    })
    .map(|_| ())
}
fn runtime_root<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|path| path.join("agent-runtimes"))
        .map_err(|error| error.to_string())
}
fn node_install_root(root: &Path) -> PathBuf {
    root.join("node")
        .join(format!("v{NODE_VERSION}-{NODE_TARGET}"))
}
fn pnpm_install_root(root: &Path) -> PathBuf {
    root.join("pnpm").join(format!("v{PNPM_VERSION}"))
}
fn valid_version(version: &str) -> bool {
    if version.is_empty() || version.len() > 128 {
        return false;
    }
    let (release, build) = version
        .split_once('+')
        .map_or((version, None), |(a, b)| (a, Some(b)));
    let (core, pre) = release
        .split_once('-')
        .map_or((release, None), |(a, b)| (a, Some(b)));
    let numeric = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part.len() == 1 || !part.starts_with('0'))
    };
    let core: Vec<_> = core.split('.').collect();
    if core.len() != 3 || !core.into_iter().all(numeric) {
        return false;
    }
    let identifiers = |value: &str, prerelease: bool| {
        value.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && (!prerelease || !part.bytes().all(|b| b.is_ascii_digit()) || numeric(part))
        })
    };
    pre.is_none_or(|value| identifiers(value, true))
        && build.is_none_or(|value| identifiers(value, false))
}
fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let release = |s: &str| s.split('+').next().unwrap_or("").to_owned();
    let a = release(a);
    let b = release(b);
    let split = |s: &str| {
        let (core, pre) = s.split_once('-').map_or((s, None), |(a, b)| (a, Some(b)));
        (
            core.split('.').map(str::to_owned).collect::<Vec<_>>(),
            pre.map(str::to_owned),
        )
    };
    let (a, ap) = split(&a);
    let (b, bp) = split(&b);
    let number = |a: &str, b: &str| a.len().cmp(&b.len()).then_with(|| a.cmp(b));
    for (a, b) in a.iter().zip(&b) {
        let cmp = number(a, b);
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    match (ap, bp) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => {
            let a: Vec<_> = a.split('.').collect();
            let b: Vec<_> = b.split('.').collect();
            for (a, b) in a.iter().zip(&b) {
                let an = a.bytes().all(|c| c.is_ascii_digit());
                let bn = b.bytes().all(|c| c.is_ascii_digit());
                let cmp = match (an, bn) {
                    (true, true) => number(a, b),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => a.cmp(b),
                };
                if cmp != Ordering::Equal {
                    return cmp;
                }
            }
            a.len().cmp(&b.len())
        }
    }
}
fn validate_registry_entry(bytes: &[u8], kind: AgentKind) -> Result<String, String> {
    if bytes.len() > REGISTRY_MAX_BYTES {
        return Err("official ACP Registry response exceeds the supported size".into());
    }
    let registry: RegistryIndex =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if registry.version != REGISTRY_SCHEMA_VERSION {
        return Err("unsupported ACP Registry schema".into());
    }
    let provider = provider(kind);
    let mut entries = registry
        .agents
        .into_iter()
        .filter(|agent| agent.id == provider.registry_id);
    let entry = entries
        .next()
        .ok_or("Agent is absent from official ACP Registry")?;
    if entries.next().is_some() || !valid_version(&entry.version) {
        return Err("invalid or duplicate official ACP Registry entry".into());
    }
    let distribution = entry
        .distribution
        .npx
        .ok_or("Agent has no supported npm distribution")?;
    if !distribution.args.is_empty()
        || !distribution.env.is_empty()
        || distribution.package != format!("{}@{}", provider.adapter_name, entry.version)
    {
        return Err("official ACP Registry package identity mismatch".into());
    }
    Ok(entry.version)
}
async fn fetch_registry_version(kind: AgentKind) -> Result<String, String> {
    let mut response = http_client()?
        .get(REGISTRY_URL)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|size| size > REGISTRY_MAX_BYTES as u64)
    {
        return Err("ACP Registry response too large".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len().saturating_add(chunk.len()) > REGISTRY_MAX_BYTES {
            return Err("ACP Registry response too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    validate_registry_entry(&bytes, kind)
}
fn manifest(kind: AgentKind, version: &str) -> Result<Vec<u8>, String> {
    if !valid_version(version) {
        return Err("invalid exact Adapter version".into());
    }
    serde_json::to_vec_pretty(&serde_json::json!({
        "name": format!("lens-managed-{}", provider(kind).registry_id), "private": true,
        "dependencies": { provider(kind).adapter_name: version },
        "engines": { "node": NODE_VERSION }, "packageManager": format!("pnpm@{PNPM_VERSION}")
    }))
    .map_err(|error| error.to_string())
}
fn read_record(path: &Path, kind: AgentKind) -> Result<InstallRecord, String> {
    let record: InstallRecord = serde_json::from_slice(
        &fs::read(path.join("lens-runtime.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let provider = provider(kind);
    if ![5, AGENT_INSTALL_RECORD_VERSION].contains(&record.schema_version)
        || record.registry_id != provider.registry_id
        || record.adapter_name != provider.adapter_name
        || !valid_version(&record.adapter_version)
        || record.node_version != NODE_VERSION
        || record.node_archive_sha256 != NODE_ARCHIVE_SHA256
        || record.pnpm_version != PNPM_VERSION
        || record.pnpm_archive_sha512 != PNPM_ARCHIVE_SHA512
    {
        return Err("managed Agent install record does not match runtime policy".into());
    }
    let package = fs::read(path.join("package.json")).map_err(|error| error.to_string())?;
    let json: serde_json::Value =
        serde_json::from_slice(&package).map_err(|error| error.to_string())?;
    if json["dependencies"][provider.adapter_name].as_str() != Some(&record.adapter_version)
        || sha256_file(&path.join("pnpm-lock.yaml"), "Agent dependency lock")?
            != record.pnpm_lock_sha256
        || sha256_file(&path.join("pnpm-workspace.yaml"), "Agent workspace policy")?
            != record.pnpm_workspace_sha256
    {
        return Err("managed Agent dependency policy was modified".into());
    }
    if record.schema_version == AGENT_INSTALL_RECORD_VERSION
        && (record.package_json_sha256.as_deref() != Some(&sha256_bytes(&package))
            || package != manifest(kind, &record.adapter_version)?
            || fs::read(path.join("pnpm-workspace.yaml")).map_err(|error| error.to_string())?
                != AGENT_WORKSPACE)
    {
        return Err("managed Agent install policy was modified".into());
    }
    Ok(record)
}
fn package_directory(root: &Path, from: &Path, package: &str) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
    let mut cursor = fs::canonicalize(from).map_err(|error| error.to_string())?;
    while cursor.starts_with(&root) {
        let candidate = cursor.join("node_modules").join(package);
        if candidate.join("package.json").is_file() {
            let file = canonical_managed_file(
                &root,
                &candidate.join("package.json"),
                "Agent dependency manifest",
            )?;
            return file
                .parent()
                .map(Path::to_owned)
                .ok_or("invalid dependency path".into());
        }
        if !cursor.pop() {
            break;
        }
    }
    Err(format!("Agent dependency {package} is missing"))
}
fn package_json(root: &Path) -> Result<serde_json::Value, String> {
    serde_json::from_slice(&fs::read(root.join("package.json")).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}
fn dependency(root: &Path, from: &Path, name: &str) -> Result<PathBuf, String> {
    let package = package_json(from)?;
    if package["dependencies"].get(name).is_none()
        && package["optionalDependencies"].get(name).is_none()
    {
        return Err(format!("Agent does not declare required dependency {name}"));
    }
    package_directory(root, from, name)
}
async fn verify_runtime_paths(
    node_root: &Path,
    agent_root: &Path,
    kind: AgentKind,
    version: &str,
) -> Result<ResolvedAgentRuntime, String> {
    verify_node_runtime(node_root).await?;
    let policy = provider(kind);
    let node = canonical_managed_file(node_root, &node_root.join("bin/node"), "Node runtime")?;
    let adapter = package_directory(agent_root, agent_root, policy.adapter_name)?;
    let package = package_json(&adapter)?;
    if package["name"].as_str() != Some(policy.adapter_name)
        || package["version"].as_str() != Some(version)
    {
        return Err("installed Adapter identity mismatch".into());
    }
    let bin = package["bin"][policy.bin_name]
        .as_str()
        .ok_or("Adapter executable entry is missing")?;
    let entrypoint = canonical_managed_file(agent_root, &adapter.join(bin), "ACP adapter")?;
    let paths = match kind {
        AgentKind::Claude => {
            let sdk = dependency(agent_root, &adapter, "@anthropic-ai/claude-agent-sdk")?;
            let platform = dependency(
                agent_root,
                &sdk,
                "@anthropic-ai/claude-agent-sdk-darwin-arm64",
            )?;
            vec![(
                platform.join("claude"),
                CLAUDE_TEAM_ID,
                "com.anthropic.claude-code",
            )]
        }
        AgentKind::Codex => {
            let cli = dependency(agent_root, &adapter, "@openai/codex")?;
            let platform = dependency(agent_root, &cli, "@openai/codex-darwin-arm64")?;
            vec![
                (
                    platform.join("vendor/aarch64-apple-darwin/bin/codex"),
                    OPENAI_TEAM_ID,
                    "codex",
                ),
                (
                    platform.join("vendor/aarch64-apple-darwin/bin/codex-code-mode-host"),
                    OPENAI_TEAM_ID,
                    "codex-code-mode-host",
                ),
            ]
        }
    };
    for (path, team, identifier) in paths {
        let path = canonical_managed_file(agent_root, &path, "Agent native executable")?;
        verify_code_signature(&path, team, identifier, "Agent native executable").await?;
    }
    let expected = match kind {
        AgentKind::Claude => version.to_owned(),
        AgentKind::Codex => format!("{} {version}", policy.adapter_name),
    };
    verify_version_command(
        &node,
        &[entrypoint.to_string_lossy().as_ref(), "--version"],
        &expected,
        "ACP adapter",
    )
    .await?;
    Ok(ResolvedAgentRuntime {
        kind: policy.kind,
        adapter_name: policy.adapter_name,
        adapter_version: version.into(),
        safe_mode_id: policy.safe_mode_id,
        command: node,
        args: vec![entrypoint.to_string_lossy().into_owned()],
        installation: None,
    })
}
async fn load_runtime(
    root: &Path,
    kind: AgentKind,
    id: &str,
) -> Result<ResolvedAgentRuntime, String> {
    let lease = {
        let _guard = selector_mutex()
            .lock()
            .map_err(|_| "runtime selector unavailable")?;
        acquire_lease(root, kind, id)?
    };
    let path = install_root(root, kind, id)?;
    if !fs::symlink_metadata(&path)
        .map_err(|error| error.to_string())?
        .is_dir()
    {
        return Err("managed installation is not a directory".into());
    }
    let record = read_record(&path, kind)?;
    let mut runtime = verify_runtime_paths(
        &node_install_root(root),
        &path,
        kind,
        &record.adapter_version,
    )
    .await?;
    runtime.installation = Some(lease);
    Ok(runtime)
}
async fn migrate_legacy(root: &Path, kind: AgentKind) -> Result<(), String> {
    let selector = read_selector(root, kind)?;
    if selector.current.is_some() || selector.candidate.is_some() {
        return Ok(());
    }
    let dir = provider_root(root, kind);
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(&dir)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|ty| ty.is_dir())
                && valid_version(&entry.file_name().to_string_lossy())
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        version_order(
            &a.file_name().to_string_lossy(),
            &b.file_name().to_string_lossy(),
        )
    });
    for entry in entries.into_iter().rev() {
        let path = entry.path();
        let Ok(record) = read_record(&path, kind) else {
            continue;
        };
        if record.schema_version != 5
            || record.adapter_version != entry.file_name().to_string_lossy()
        {
            continue;
        }
        if verify_runtime_paths(
            &node_install_root(root),
            &path,
            kind,
            &record.adapter_version,
        )
        .await
        .is_err()
        {
            continue;
        }
        let _guard = selector_mutex()
            .lock()
            .map_err(|_| "runtime selector unavailable")?;
        let mut selector = read_selector(root, kind)?;
        if selector.current.is_none() {
            selector.current = Some(format!("legacy:{}", record.adapter_version));
            write_selector(root, kind, &selector)?;
        }
        return Ok(());
    }
    Ok(())
}

async fn ensure_node_runtime<R: tauri::Runtime>(
    app: &AppHandle<R>,
    root: &Path,
    operation_id: Uuid,
) -> Result<PathBuf, String> {
    let final_root = node_install_root(root);
    if final_root.is_dir() {
        match verify_node_runtime(&final_root).await {
            Ok(()) => return Ok(final_root),
            Err(_) => quarantine_existing(root, &final_root, "node")?,
        }
    }

    update_agent_runtime(app, operation_id, |runtime| {
        runtime.stage = AgentRuntimeStage::Downloading;
        runtime.message = Some(format!("Downloading Node.js {NODE_VERSION}…"));
        runtime.downloaded_bytes = 0;
        runtime.total_bytes = None;
    })?;

    let staging_root = root.join(".staging").join(Uuid::new_v4().to_string());
    let archive_path = staging_root.join(NODE_ARCHIVE_NAME);
    fs::create_dir_all(&staging_root)
        .map_err(|error| format!("unable to create Node staging directory: {error}"))?;
    let result = async {
        download_node_archive(app, operation_id, &archive_path).await?;
        update_agent_runtime(app, operation_id, |runtime| {
            runtime.stage = AgentRuntimeStage::Verifying;
            runtime.message = Some("Verifying and extracting Node.js…".into());
        })?;
        let extraction_root = staging_root.join("extracted");
        extract_approved_archive(
            &archive_path,
            &extraction_root,
            NODE_ARCHIVE_ROOT,
            "Node.js",
        )
        .await?;
        let extracted_node = extraction_root.join(NODE_ARCHIVE_ROOT);
        verify_node_runtime_payload(&extracted_node).await?;
        let node_record = NodeInstallRecord {
            schema_version: NODE_INSTALL_RECORD_VERSION,
            node_version: NODE_VERSION.into(),
            node_target: NODE_TARGET.into(),
            archive_sha256: NODE_ARCHIVE_SHA256.into(),
        };
        let node_record = serde_json::to_vec_pretty(&node_record)
            .map_err(|error| format!("unable to serialize Node install record: {error}"))?;
        fs::write(extracted_node.join("lens-node-runtime.json"), node_record)
            .map_err(|error| format!("unable to write Node install record: {error}"))?;
        verify_node_runtime(&extracted_node).await?;
        let parent = final_root
            .parent()
            .ok_or_else(|| "managed Node path has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("unable to create managed Node directory: {error}"))?;
        match fs::rename(&extracted_node, &final_root) {
            Ok(()) => Ok(final_root.clone()),
            Err(_error) if final_root.is_dir() => {
                verify_node_runtime(&final_root).await?;
                Ok(final_root.clone())
            }
            Err(error) => Err(format!("unable to activate managed Node runtime: {error}")),
        }
    }
    .await;
    cleanup_staging(root, &staging_root);
    result
}

async fn verify_node_runtime(node_root: &Path) -> Result<(), String> {
    let record_path = node_root.join("lens-node-runtime.json");
    let record_bytes = fs::read(&record_path)
        .map_err(|error| format!("managed Node install record is unavailable: {error}"))?;
    let record = serde_json::from_slice::<NodeInstallRecord>(&record_bytes)
        .map_err(|error| format!("managed Node install record is invalid: {error}"))?;
    if record.schema_version != NODE_INSTALL_RECORD_VERSION
        || record.node_version != NODE_VERSION
        || record.node_target != NODE_TARGET
        || record.archive_sha256 != NODE_ARCHIVE_SHA256
    {
        return Err("managed Node runtime does not match the approved install record".into());
    }
    verify_node_runtime_payload(node_root).await
}

async fn verify_node_runtime_payload(node_root: &Path) -> Result<(), String> {
    let node = canonical_managed_file(node_root, &node_root.join("bin/node"), "Node runtime")?;
    verify_code_signature(&node, NODE_TEAM_ID, NODE_SIGNING_IDENTIFIER, "Node runtime").await?;
    verify_version_command(&node, &["--version"], &format!("v{NODE_VERSION}"), "Node").await
}

async fn download_node_archive<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    destination: &Path,
) -> Result<(), String> {
    let client = http_client()?;
    let mut response = client
        .get(NODE_ARCHIVE_URL)
        .send()
        .await
        .map_err(|error| format!("unable to download Node.js: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Node.js download returned an error: {error}"))?;
    let total = response.content_length();
    if total.is_some_and(|size| size > NODE_ARCHIVE_MAX_BYTES) {
        return Err("Node.js archive exceeds the approved size limit".into());
    }
    update_agent_runtime(app, operation_id, |runtime| runtime.total_bytes = total)?;

    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|error| format!("unable to create Node.js download: {error}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut last_published = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("unable to read Node.js download: {error}"))?
    {
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| "Node.js download size overflow".to_string())?;
        if downloaded > NODE_ARCHIVE_MAX_BYTES {
            return Err("Node.js archive exceeds the approved size limit".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("unable to write Node.js download: {error}"))?;
        hasher.update(&chunk);
        if downloaded.saturating_sub(last_published) >= 1024 * 1024 {
            update_agent_runtime(app, operation_id, |runtime| {
                runtime.downloaded_bytes = downloaded
            })?;
            last_published = downloaded;
        }
    }
    file.flush()
        .await
        .map_err(|error| format!("unable to flush Node.js download: {error}"))?;
    update_agent_runtime(app, operation_id, |runtime| {
        runtime.downloaded_bytes = downloaded
    })?;
    let digest = hasher.finalize();
    let actual = hex_digest(&digest);
    if actual != NODE_ARCHIVE_SHA256 {
        return Err(format!(
            "Node.js archive checksum mismatch: expected {NODE_ARCHIVE_SHA256}, got {actual}"
        ));
    }
    Ok(())
}

async fn extract_approved_archive(
    archive: &Path,
    destination: &Path,
    expected_root: &'static str,
    label: &'static str,
) -> Result<(), String> {
    let archive = archive.to_path_buf();
    let destination = destination.to_path_buf();
    tokio::task::spawn_blocking(move || {
        extract_approved_archive_blocking(&archive, &destination, expected_root, label)
    })
    .await
    .map_err(|error| format!("{label} extraction task failed: {error}"))?
}

fn extract_approved_archive_blocking(
    archive: &Path,
    destination: &Path,
    expected_root: &str,
    label: &str,
) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("unable to create {label} extraction directory: {error}"))?;
    let file =
        File::open(archive).map_err(|error| format!("unable to open {label} archive: {error}"))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| format!("unable to read {label} archive: {error}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| format!("invalid {label} archive entry: {error}"))?;
        let path = entry
            .path()
            .map_err(|error| format!("invalid {label} archive path: {error}"))?
            .into_owned();
        validate_archive_path(&path, expected_root)?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() && !entry_type.is_symlink() {
            return Err(format!(
                "{label} archive contains an unsupported entry type: {}",
                path.display()
            ));
        }
        if let Some(link) = entry
            .link_name()
            .map_err(|error| format!("invalid {label} archive link: {error}"))?
        {
            if !entry_type.is_symlink() {
                return Err(format!(
                    "{label} archive contains an unsupported link type: {}",
                    path.display()
                ));
            }
            validate_archive_link(&path, &link, expected_root)?;
        }
        if !entry
            .unpack_in(destination)
            .map_err(|error| format!("unable to extract {label} archive entry: {error}"))?
        {
            return Err(format!(
                "{label} archive entry escapes the extraction directory"
            ));
        }
    }
    Ok(())
}

async fn ensure_pnpm_runtime<R: tauri::Runtime>(
    app: &AppHandle<R>,
    root: &Path,
    node_root: &Path,
    operation_id: Uuid,
) -> Result<PathBuf, String> {
    let final_root = pnpm_install_root(root);
    if final_root.is_dir() {
        match verify_pnpm_runtime(node_root, &final_root).await {
            Ok(()) => return Ok(final_root),
            Err(_) => quarantine_existing(root, &final_root, "pnpm")?,
        }
    }

    update_agent_runtime(app, operation_id, |runtime| {
        runtime.stage = AgentRuntimeStage::Downloading;
        runtime.message = Some(format!(
            "Downloading pnpm {PNPM_VERSION} from Takumi Guard…"
        ));
        runtime.downloaded_bytes = 0;
        runtime.total_bytes = None;
    })?;

    let staging_root = root.join(".staging").join(Uuid::new_v4().to_string());
    let archive_path = staging_root.join(PNPM_ARCHIVE_NAME);
    fs::create_dir_all(&staging_root)
        .map_err(|error| format!("unable to create pnpm staging directory: {error}"))?;
    let result = async {
        download_pnpm_archive(app, operation_id, &archive_path).await?;
        update_agent_runtime(app, operation_id, |runtime| {
            runtime.stage = AgentRuntimeStage::Verifying;
            runtime.message = Some("Verifying and extracting pnpm…".into());
        })?;
        let extraction_root = staging_root.join("extracted");
        extract_approved_archive(&archive_path, &extraction_root, PNPM_ARCHIVE_ROOT, "pnpm")
            .await?;
        let extracted_pnpm = extraction_root.join(PNPM_ARCHIVE_ROOT);
        verify_pnpm_runtime_payload(node_root, &extracted_pnpm).await?;
        let pnpm_record = PnpmInstallRecord {
            schema_version: PNPM_INSTALL_RECORD_VERSION,
            pnpm_version: PNPM_VERSION.into(),
            registry: TAKUMI_GUARD_REGISTRY.into(),
            archive_sha512: PNPM_ARCHIVE_SHA512.into(),
            cli_sha256: PNPM_CLI_SHA256.into(),
            dist_sha256: PNPM_DIST_SHA256.into(),
        };
        let pnpm_record = serde_json::to_vec_pretty(&pnpm_record)
            .map_err(|error| format!("unable to serialize pnpm install record: {error}"))?;
        fs::write(extracted_pnpm.join("lens-pnpm-runtime.json"), pnpm_record)
            .map_err(|error| format!("unable to write pnpm install record: {error}"))?;
        verify_pnpm_runtime(node_root, &extracted_pnpm).await?;
        let parent = final_root
            .parent()
            .ok_or_else(|| "managed pnpm path has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("unable to create managed pnpm directory: {error}"))?;
        match fs::rename(&extracted_pnpm, &final_root) {
            Ok(()) => Ok(final_root.clone()),
            Err(_error) if final_root.is_dir() => {
                verify_pnpm_runtime(node_root, &final_root).await?;
                Ok(final_root.clone())
            }
            Err(error) => Err(format!("unable to activate managed pnpm runtime: {error}")),
        }
    }
    .await;
    cleanup_staging(root, &staging_root);
    result
}

async fn verify_pnpm_runtime(node_root: &Path, pnpm_root: &Path) -> Result<(), String> {
    let record_path = pnpm_root.join("lens-pnpm-runtime.json");
    let record_bytes = fs::read(&record_path)
        .map_err(|error| format!("managed pnpm install record is unavailable: {error}"))?;
    let record = serde_json::from_slice::<PnpmInstallRecord>(&record_bytes)
        .map_err(|error| format!("managed pnpm install record is invalid: {error}"))?;
    if record.schema_version != PNPM_INSTALL_RECORD_VERSION
        || record.pnpm_version != PNPM_VERSION
        || record.registry != TAKUMI_GUARD_REGISTRY
        || record.archive_sha512 != PNPM_ARCHIVE_SHA512
        || record.cli_sha256 != PNPM_CLI_SHA256
        || record.dist_sha256 != PNPM_DIST_SHA256
    {
        return Err("managed pnpm runtime does not match the approved install record".into());
    }
    verify_pnpm_runtime_payload(node_root, pnpm_root).await
}

async fn verify_pnpm_runtime_payload(node_root: &Path, pnpm_root: &Path) -> Result<(), String> {
    let node = canonical_managed_file(node_root, &node_root.join("bin/node"), "Node runtime")?;
    let pnpm_cli = canonical_managed_file(pnpm_root, &pnpm_root.join("bin/pnpm.mjs"), "pnpm CLI")?;
    let pnpm_dist = canonical_managed_file(
        pnpm_root,
        &pnpm_root.join("dist/pnpm.mjs"),
        "pnpm distribution",
    )?;
    if sha256_file(&pnpm_cli, "pnpm CLI")? != PNPM_CLI_SHA256
        || sha256_file(&pnpm_dist, "pnpm distribution")? != PNPM_DIST_SHA256
    {
        return Err("managed pnpm executable files were modified".into());
    }
    verify_version_command(
        &node,
        &[
            pnpm_cli.to_string_lossy().as_ref(),
            "--pm-on-fail=error",
            "--version",
        ],
        PNPM_VERSION,
        "pnpm",
    )
    .await
}

async fn download_pnpm_archive<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation_id: Uuid,
    destination: &Path,
) -> Result<(), String> {
    let client = http_client()?;
    let mut response = client
        .get(PNPM_ARCHIVE_URL)
        .send()
        .await
        .map_err(|error| format!("unable to download pnpm from Takumi Guard: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Takumi Guard pnpm download returned an error: {error}"))?;
    let total = response.content_length();
    if total.is_some_and(|size| size > PNPM_ARCHIVE_MAX_BYTES) {
        return Err("pnpm archive exceeds the approved size limit".into());
    }
    update_agent_runtime(app, operation_id, |runtime| runtime.total_bytes = total)?;

    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|error| format!("unable to create pnpm download: {error}"))?;
    let mut hasher = Sha512::new();
    let mut downloaded = 0_u64;
    let mut last_published = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("unable to read pnpm download: {error}"))?
    {
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| "pnpm download size overflow".to_string())?;
        if downloaded > PNPM_ARCHIVE_MAX_BYTES {
            return Err("pnpm archive exceeds the approved size limit".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("unable to write pnpm download: {error}"))?;
        hasher.update(&chunk);
        if downloaded.saturating_sub(last_published) >= 1024 * 1024 {
            update_agent_runtime(app, operation_id, |runtime| {
                runtime.downloaded_bytes = downloaded
            })?;
            last_published = downloaded;
        }
    }
    file.flush()
        .await
        .map_err(|error| format!("unable to flush pnpm download: {error}"))?;
    update_agent_runtime(app, operation_id, |runtime| {
        runtime.downloaded_bytes = downloaded
    })?;
    let digest = hasher.finalize();
    let actual = hex_digest(&digest);
    if actual != PNPM_ARCHIVE_SHA512 {
        return Err(format!(
            "pnpm archive checksum mismatch: expected {PNPM_ARCHIVE_SHA512}, got {actual}"
        ));
    }
    Ok(())
}

struct StagingCleanup {
    root: PathBuf,
    path: PathBuf,
}
impl Drop for StagingCleanup {
    fn drop(&mut self) {
        cleanup_staging(&self.root, &self.path);
    }
}

#[derive(Clone, Copy)]
enum CandidateRequest<'a> {
    #[cfg(test)]
    Exact(&'a str),
    Eligible {
        ceiling: &'a str,
        after: Option<&'a str>,
    },
}
impl<'a> CandidateRequest<'a> {
    fn ceiling(self) -> &'a str {
        match self {
            #[cfg(test)]
            Self::Exact(version) => version,
            Self::Eligible { ceiling, .. } => ceiling,
        }
    }
    fn selector(self) -> Result<Option<String>, String> {
        if !valid_version(self.ceiling()) {
            return Err("invalid registry version ceiling".into());
        }
        match self {
            #[cfg(test)]
            Self::Exact(_) => Ok(None),
            Self::Eligible { ceiling, after } => {
                if let Some(after) = after {
                    if !valid_version(after)
                        || version_order(after, ceiling) != std::cmp::Ordering::Less
                    {
                        return Err("no newer registry candidate is available".into());
                    }
                }
                Ok(Some(after.map_or_else(
                    || format!("<={ceiling}"),
                    |after| format!(">{after} <={ceiling}"),
                )))
            }
        }
    }
    fn accepts(self, version: &str) -> bool {
        valid_version(version)
            && match self {
                #[cfg(test)]
                Self::Exact(expected) => version == expected,
                Self::Eligible { ceiling, after } => {
                    version_order(version, ceiling) != std::cmp::Ordering::Greater
                        && after.is_none_or(|after| {
                            version_order(version, after) == std::cmp::Ordering::Greater
                        })
                }
            }
    }
}

fn resolved_candidate_manifest(
    bytes: &[u8],
    kind: AgentKind,
    request: CandidateRequest<'_>,
) -> Result<(String, Vec<u8>), String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let version = value["dependencies"][provider(kind).adapter_name]
        .as_str()
        .filter(|version| request.accepts(version))
        .ok_or("resolved Agent version is not an exact permitted registry candidate")?
        .to_owned();
    let canonical = manifest(kind, &version)?;
    if value
        != serde_json::from_slice::<serde_json::Value>(&canonical)
            .map_err(|error| error.to_string())?
    {
        return Err("dependency resolution modified the managed Agent manifest".into());
    }
    Ok((version, canonical))
}

#[cfg(test)]
async fn install_candidate<R: tauri::Runtime>(
    app: &AppHandle<R>,
    root: &Path,
    node: &Path,
    pnpm: &Path,
    kind: AgentKind,
    version: &str,
    operation: Uuid,
) -> Result<ResolvedAgentRuntime, String> {
    install_requested_candidate(
        app,
        root,
        node,
        pnpm,
        kind,
        CandidateRequest::Exact(version),
        operation,
    )
    .await
}

async fn install_requested_candidate<R: tauri::Runtime>(
    app: &AppHandle<R>,
    root: &Path,
    node: &Path,
    pnpm: &Path,
    kind: AgentKind,
    request: CandidateRequest<'_>,
    operation: Uuid,
) -> Result<ResolvedAgentRuntime, String> {
    let version = request.ceiling();
    let selector = request.selector()?;
    update_agent_runtime(app, operation, |state| {
        state.stage = AgentRuntimeStage::Installing;
        state.message = Some(format!(
            "Resolving {} up to {version} through Takumi Guard and Safe-chain…",
            display_name(kind)
        ));
    })?;
    let id = Uuid::new_v4().to_string();
    let staging = root.join(".staging").join(&id);
    let agent = staging.join("agent");
    fs::create_dir_all(&agent).map_err(|error| error.to_string())?;
    let _cleanup = StagingCleanup {
        root: root.to_owned(),
        path: staging.clone(),
    };
    let package = manifest(kind, version)?;
    fs::write(agent.join("package.json"), &package).map_err(|error| error.to_string())?;
    fs::write(agent.join("pnpm-workspace.yaml"), AGENT_WORKSPACE)
        .map_err(|error| error.to_string())?;
    fs::write(staging.join("blank-user-npmrc"), []).map_err(|error| error.to_string())?;
    fs::write(staging.join("blank-global-npmrc"), []).map_err(|error| error.to_string())?;
    let result = async {
        let specification = selector
            .as_ref()
            .map(|selector| format!("{}@{selector}", provider(kind).adapter_name));
        let mode = specification.as_deref().map_or(
            PnpmInstallMode::ResolveExact,
            PnpmInstallMode::ResolveEligible,
        );
        run_pnpm_install(node, pnpm, &agent, &staging, mode).await?;
        let resolved_manifest =
            fs::read(agent.join("package.json")).map_err(|error| error.to_string())?;
        let (version, package) = resolved_candidate_manifest(&resolved_manifest, kind, request)?;
        if read_selector(root, kind)?.rejected_version.as_deref() == Some(&version) {
            return Err(format!(
                "{} {version} previously failed required ACP compatibility checks",
                display_name(kind)
            ));
        }
        // pnpm add saves the selected exact version in both manifest and lock. Canonicalize
        // formatting only after validating every field, then keep both inputs frozen.
        fs::write(agent.join("package.json"), &package).map_err(|error| error.to_string())?;
        update_agent_runtime(app, operation, |state| {
            state.version = Some(version.clone());
            state.message = Some(format!(
                "Installing {} {version}, the newest release allowed by the installation policy…",
                display_name(kind)
            ));
        })?;
        let lock = fs::read(agent.join("pnpm-lock.yaml")).map_err(|error| error.to_string())?;
        if lock.is_empty() {
            return Err("resolved dependency lock is empty".into());
        }
        if fs::read(agent.join("package.json")).map_err(|error| error.to_string())? != package
            || fs::read(agent.join("pnpm-workspace.yaml")).map_err(|error| error.to_string())?
                != AGENT_WORKSPACE
        {
            return Err("dependency resolution modified install policy".into());
        }
        run_pnpm_install(node, pnpm, &agent, &staging, PnpmInstallMode::Frozen).await?;
        if fs::read(agent.join("package.json")).map_err(|error| error.to_string())? != package
            || fs::read(agent.join("pnpm-workspace.yaml")).map_err(|error| error.to_string())?
                != AGENT_WORKSPACE
            || fs::read(agent.join("pnpm-lock.yaml")).map_err(|error| error.to_string())? != lock
        {
            return Err(
                "Agent installation modified its manifest, lock or Safe-chain policy".into(),
            );
        }
        verify_runtime_paths(node, &agent, kind, &version).await?;
        let policy = provider(kind);
        let record = InstallRecord {
            schema_version: AGENT_INSTALL_RECORD_VERSION,
            registry_id: policy.registry_id.into(),
            adapter_name: policy.adapter_name.into(),
            adapter_version: version,
            node_version: NODE_VERSION.into(),
            node_archive_sha256: NODE_ARCHIVE_SHA256.into(),
            pnpm_version: PNPM_VERSION.into(),
            pnpm_archive_sha512: PNPM_ARCHIVE_SHA512.into(),
            pnpm_lock_sha256: sha256_bytes(&lock),
            pnpm_workspace_sha256: sha256_bytes(AGENT_WORKSPACE),
            package_json_sha256: Some(sha256_bytes(&package)),
        };
        fs::write(
            agent.join("lens-runtime.json"),
            serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let destination = install_root(root, kind, &id)?;
        fs::create_dir_all(destination.parent().unwrap()).map_err(|error| error.to_string())?;
        {
            let _guard = selector_mutex()
                .lock()
                .map_err(|_| "runtime selector unavailable")?;
            fs::rename(&agent, &destination).map_err(|error| error.to_string())?;
            let mut selector = read_selector(root, kind)?;
            selector.candidate = Some(id.clone());
            write_selector(root, kind, &selector)?;
        }
        load_runtime(root, kind, &id).await
    }
    .await;
    cleanup_staging(root, &staging);
    result
}
fn package_manager_override(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("npm_config_")
        || key.starts_with("pnpm_")
        || key.starts_with("corepack_")
        || matches!(
            key.as_str(),
            "node_options" | "node_path" | "node_tls_reject_unauthorized"
        )
}
enum PnpmInstallMode<'a> {
    ResolveExact,
    ResolveEligible(&'a str),
    Frozen,
}

async fn run_pnpm_install(
    node_root: &Path,
    pnpm_root: &Path,
    agent: &Path,
    staging: &Path,
    mode: PnpmInstallMode<'_>,
) -> Result<(), String> {
    let node = canonical_managed_file(node_root, &node_root.join("bin/node"), "Node runtime")?;
    let pnpm = canonical_managed_file(pnpm_root, &pnpm_root.join("bin/pnpm.mjs"), "pnpm CLI")?;
    let mut command = Command::new(node);
    for (key, _) in std::env::vars_os() {
        if package_manager_override(&key.to_string_lossy()) {
            command.env_remove(key);
        }
    }
    command
        .arg(pnpm)
        .arg(match mode {
            PnpmInstallMode::ResolveEligible(_) => "add",
            _ => "install",
        })
        .args([
            "--prod",
            "--pm-on-fail=error",
            "--registry=https://npm.flatt.tech/",
        ]);
    match mode {
        PnpmInstallMode::ResolveExact => {
            command.args(["--lockfile-only", "--no-frozen-lockfile"]);
        }
        PnpmInstallMode::ResolveEligible(specification) => {
            command.args(["--lockfile-only", "--save-exact", specification]);
        }
        PnpmInstallMode::Frozen => {
            command.arg("--frozen-lockfile");
        }
    }
    command
        .arg(format!(
            "--store-dir={}",
            staging.join("pnpm-store").display()
        ))
        .current_dir(agent)
        .env("CI", "true")
        .env("NODE_OPTIONS", "")
        .env("NODE_PATH", "")
        .env("NODE_TLS_REJECT_UNAUTHORIZED", "1")
        .env("PNPM_HOME", "")
        .env("COREPACK_HOME", "")
        .env("NPM_CONFIG_USERCONFIG", staging.join("blank-user-npmrc"))
        .env(
            "NPM_CONFIG_GLOBALCONFIG",
            staging.join("blank-global-npmrc"),
        )
        .env("npm_config_node_options", "")
        .env("npm_config_update_notifier", "false")
        .env("npm_config_registry", TAKUMI_GUARD_REGISTRY)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(600), command.output())
        .await
        .map_err(|_| "managed Agent pnpm installation timed out".to_string())?
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "managed Agent Safe-chain installation failed: {} {}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}
pub(crate) fn publish_recovery<R: tauri::Runtime>(
    app: &AppHandle<R>,
    runtime: &ResolvedAgentRuntime,
    reason: &str,
) -> Result<(), String> {
    publish_agent_runtime(app, AgentRuntimeState {
        operation_id: Some(Uuid::new_v4()), agent: Some(runtime.kind), stage: AgentRuntimeStage::Ready,
        version: Some(runtime.adapter_version.clone()), message: Some(format!("The update did not meet required ACP capabilities. Continuing with verified {} {}.", display_name(runtime.kind), runtime.adapter_version)),
        error: Some(format!("Agent update failed required startup compatibility checks: {reason}")), ..Default::default()
    })
}

async fn verify_code_signature(
    path: &Path,
    team_id: &str,
    signing_identifier: &str,
    label: &str,
) -> Result<(), String> {
    let requirement = format!(
        "=anchor apple generic and certificate leaf[subject.OU] = \"{team_id}\" and identifier \"{signing_identifier}\""
    );
    let output = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--test-requirement"])
        .arg(requirement)
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| format!("unable to verify {label} code signature: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{label} code signature is not issued by the approved publisher: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

async fn verify_version_command(
    command: &Path,
    args: &[&str],
    expected: &str,
    label: &str,
) -> Result<(), String> {
    let mut process = Command::new(command);
    for (key, _) in std::env::vars_os() {
        if package_manager_override(&key.to_string_lossy()) {
            process.env_remove(key);
        }
    }
    let future = process
        .current_dir(command.parent().ok_or("version command has no parent")?)
        .env("NPM_CONFIG_USERCONFIG", "/dev/null")
        .env("NPM_CONFIG_GLOBALCONFIG", "/dev/null")
        .kill_on_drop(true)
        .args(args)
        .env("NODE_OPTIONS", "")
        .env("NODE_PATH", "")
        .env("PNPM_HOME", "")
        .env("COREPACK_HOME", "")
        .stdin(Stdio::null())
        .output();
    let output = tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .map_err(|_| format!("{label} version check timed out"))?
        .map_err(|error| format!("unable to run {label} version check: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || stdout.trim() != expected {
        return Err(format!(
            "{label} version check did not report exactly {expected}: {}{}",
            stdout.trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn canonical_managed_file(root: &Path, path: &Path, label: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("unable to resolve managed {label} root: {error}"))?;
    let path = path
        .canonicalize()
        .map_err(|error| format!("{label} is missing at {}: {error}", path.display()))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(format!(
            "{label} must be a regular file inside its managed runtime directory"
        ));
    }
    Ok(path)
}

fn validate_archive_path(path: &Path, expected_root: &str) -> Result<(), String> {
    let normalized = normalize_relative(path).ok_or_else(|| {
        format!(
            "managed runtime archive contains an unsafe path: {}",
            path.display()
        )
    })?;
    if normalized.components().next() != Some(Component::Normal(expected_root.as_ref())) {
        return Err(format!(
            "managed runtime archive entry is outside the approved root: {}",
            path.display()
        ));
    }
    Ok(())
}

fn validate_archive_link(path: &Path, link: &Path, expected_root: &str) -> Result<(), String> {
    if link.is_absolute() {
        return Err(format!(
            "managed runtime archive contains an absolute link: {}",
            path.display()
        ));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let resolved = normalize_relative(&parent.join(link)).ok_or_else(|| {
        format!(
            "managed runtime archive link escapes the approved root: {}",
            path.display()
        )
    })?;
    if resolved.components().next() != Some(Component::Normal(expected_root.as_ref())) {
        return Err(format!(
            "managed runtime archive link escapes the approved root: {}",
            path.display()
        ));
    }
    Ok(())
}

fn normalize_relative(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(normalized)
}

fn quarantine_existing(root: &Path, source: &Path, label: &str) -> Result<(), String> {
    if !source.starts_with(root) || !source.exists() {
        return Err("refusing to quarantine a path outside managed runtime storage".into());
    }
    let quarantine = root.join(".quarantine");
    fs::create_dir_all(&quarantine)
        .map_err(|error| format!("unable to create runtime quarantine directory: {error}"))?;
    let destination = quarantine.join(format!("{label}-{}", Uuid::new_v4()));
    fs::rename(source, destination)
        .map_err(|error| format!("unable to quarantine invalid managed runtime: {error}"))
}

fn cleanup_staging(root: &Path, staging: &Path) {
    if staging.starts_with(root.join(".staging")) {
        let _ = fs::remove_dir_all(staging);
    }
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .user_agent(concat!("Lens/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("unable to create managed runtime HTTP client: {error}"))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    hex_digest(&digest)
}

fn sha256_file(path: &Path, label: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("unable to read {label}: {error}"))?;
    Ok(sha256_bytes(&bytes))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn ensure_supported_target() -> Result<(), String> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok(())
    } else {
        Err("the managed Agent runtime PoC supports aarch64-apple-darwin only".into())
    }
}

fn display_name(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "Claude Agent",
        AgentKind::Codex => "Codex",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn copy_fixture(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let to = destination.join(entry.file_name());
            let ty = entry.file_type().unwrap();
            if ty.is_symlink() {
                std::os::unix::fs::symlink(fs::read_link(entry.path()).unwrap(), to).unwrap();
            } else if ty.is_dir() {
                copy_fixture(&entry.path(), &to);
            } else {
                fs::copy(entry.path(), to).unwrap();
            }
        }
    }
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("lens-runtime-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn runtime(root: &Path, kind: AgentKind, id: &str) -> ResolvedAgentRuntime {
        let policy = provider(kind);
        ResolvedAgentRuntime {
            kind,
            adapter_name: policy.adapter_name,
            adapter_version: "1.2.3".into(),
            safe_mode_id: policy.safe_mode_id,
            command: PathBuf::new(),
            args: vec![],
            installation: Some(acquire_lease(root, kind, id).unwrap()),
        }
    }
    #[test]
    fn registry_requires_unique_supported_identity_and_exact_version() {
        let valid = serde_json::json!({"version":"1.0.0","agents":[{"id":"codex-acp","version":"1.2.3","distribution":{"npx":{"package":"@agentclientprotocol/codex-acp@1.2.3"}}}]});
        assert_eq!(
            validate_registry_entry(&serde_json::to_vec(&valid).unwrap(), AgentKind::Codex)
                .unwrap(),
            "1.2.3"
        );
        for field in [
            "wrong-package@1.2.3",
            "@agentclientprotocol/codex-acp@latest",
            "@agentclientprotocol/codex-acp@1.2.4",
        ] {
            let mut invalid = valid.clone();
            invalid["agents"][0]["distribution"]["npx"]["package"] = field.into();
            assert!(validate_registry_entry(
                &serde_json::to_vec(&invalid).unwrap(),
                AgentKind::Codex
            )
            .is_err());
        }
        let mut duplicate = valid.clone();
        duplicate["agents"]
            .as_array_mut()
            .unwrap()
            .push(valid["agents"][0].clone());
        assert!(validate_registry_entry(
            &serde_json::to_vec(&duplicate).unwrap(),
            AgentKind::Codex
        )
        .is_err());
        assert!(
            validate_registry_entry(&vec![0; REGISTRY_MAX_BYTES + 1], AgentKind::Codex).is_err()
        );
        assert!(!valid_version("../../escape"));
    }
    #[test]
    fn manifest_keeps_managed_package_manager_and_safe_chain_authority() {
        let json: serde_json::Value =
            serde_json::from_slice(&manifest(AgentKind::Claude, "0.99.0").unwrap()).unwrap();
        assert_eq!(
            json["dependencies"]["@agentclientprotocol/claude-agent-acp"],
            "0.99.0"
        );
        assert_eq!(json["packageManager"], format!("pnpm@{PNPM_VERSION}"));
        assert_eq!(json["engines"]["node"], NODE_VERSION);
        let policy = std::str::from_utf8(AGENT_WORKSPACE).unwrap();
        for required in [
            "minimumReleaseAge: 4320",
            "minimumReleaseAgeStrict: true",
            "trustPolicy: no-downgrade",
            "allowBuilds: {}",
            "pmOnFail: error",
        ] {
            assert!(policy.contains(required));
        }
    }
    #[test]
    fn ready_callback_promotes_only_the_exact_candidate_and_retains_one_previous() {
        let root = root();
        let kind = AgentKind::Codex;
        let old = Uuid::new_v4().to_string();
        let next = Uuid::new_v4().to_string();
        fs::create_dir_all(install_root(&root, kind, &old).unwrap()).unwrap();
        fs::create_dir_all(install_root(&root, kind, &next).unwrap()).unwrap();
        write_selector(
            &root,
            kind,
            &Selector {
                current: Some(old.clone()),
                candidate: Some(next.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let stale = runtime(&root, kind, &Uuid::new_v4().to_string());
        assert!(confirm_ready(&stale).is_err());
        let next_runtime = runtime(&root, kind, &next);
        confirm_ready(&next_runtime).unwrap();
        let selector = read_selector(&root, kind).unwrap();
        assert_eq!(selector.current, Some(next));
        assert_eq!(selector.previous, Some(old));
        assert_eq!(selector.candidate, None);
        confirm_ready(&next_runtime).unwrap();
        drop(stale);
        drop(next_runtime);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pruning_never_removes_an_installation_with_a_live_process_lease() {
        let root = root();
        let kind = AgentKind::Claude;
        let old = Uuid::new_v4().to_string();
        let current = Uuid::new_v4().to_string();
        let next = Uuid::new_v4().to_string();
        for id in [&old, &current, &next] {
            fs::create_dir_all(install_root(&root, kind, id).unwrap()).unwrap();
        }
        write_selector(
            &root,
            kind,
            &Selector {
                current: Some(current.clone()),
                previous: Some(old.clone()),
                candidate: Some(next.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let old_lease = runtime(&root, kind, &old);
        let next_lease = runtime(&root, kind, &next);
        confirm_ready(&next_lease).unwrap();
        assert!(install_root(&root, kind, &old).unwrap().exists());
        drop(old_lease);
        assert!(!install_root(&root, kind, &old).unwrap().exists());
        assert!(install_root(&root, kind, &current).unwrap().exists());
        drop(next_lease);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn incompatible_first_candidate_is_not_activated_or_replaced_by_an_arbitrary_version() {
        let root = root();
        let kind = AgentKind::Codex;
        let id = Uuid::new_v4().to_string();
        fs::create_dir_all(install_root(&root, kind, &id).unwrap()).unwrap();
        write_selector(
            &root,
            kind,
            &Selector {
                candidate: Some(id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let runtime = runtime(&root, kind, &id);
        assert!(reject_candidate(&runtime).await.unwrap().is_none());
        let selector = read_selector(&root, kind).unwrap();
        assert_eq!(selector.current, None);
        assert_eq!(selector.previous, None);
        assert_eq!(selector.candidate, None);
        assert_eq!(selector.rejected_version.as_deref(), Some("1.2.3"));
        drop(runtime);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn unconfirmed_candidate_lifecycle_preserves_all_legacy_bytes() {
        for current in [None, Some("legacy:1.2.3".to_owned())] {
            let root = root();
            let kind = AgentKind::Codex;
            let legacy = install_root(&root, kind, "legacy:1.2.3").unwrap();
            let unselected = install_root(&root, kind, "legacy:1.1.0").unwrap();
            for path in [&legacy, &unselected] {
                fs::create_dir_all(path).unwrap();
                fs::write(path.join("lens-runtime.json"), b"old schema record").unwrap();
                fs::write(path.join("adapter.js"), b"original adapter bytes").unwrap();
            }
            let id = Uuid::new_v4().to_string();
            fs::create_dir_all(install_root(&root, kind, &id).unwrap()).unwrap();
            write_selector(
                &root,
                kind,
                &Selector {
                    current,
                    candidate: Some(id.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
            // Auth failure and cancellation release the descriptor without confirming it.
            for _ in 0..2 {
                drop(runtime(&root, kind, &id));
                for path in [&legacy, &unselected] {
                    assert_eq!(
                        fs::read(path.join("adapter.js")).unwrap(),
                        b"original adapter bytes"
                    );
                }
            }
            // No current is needed to exercise rejection without loading a native fixture.
            let mut selector = read_selector(&root, kind).unwrap();
            selector.current = None;
            write_selector(&root, kind, &selector).unwrap();
            let candidate = runtime(&root, kind, &id);
            assert!(reject_candidate(&candidate).await.unwrap().is_none());
            drop(candidate);
            assert!(!install_root(&root, kind, &id).unwrap().exists());
            for path in [&legacy, &unselected] {
                assert_eq!(
                    fs::read(path.join("lens-runtime.json")).unwrap(),
                    b"old schema record"
                );
                assert_eq!(
                    fs::read(path.join("adapter.js")).unwrap(),
                    b"original adapter bytes"
                );
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn confirmed_replacement_retires_only_unreferenced_unleased_legacy_installs() {
        let root = root();
        let kind = AgentKind::Codex;
        let previous = "legacy:1.2.3";
        let leased = "legacy:1.1.0";
        let retired = "legacy:1.0.0";
        let next = Uuid::new_v4().to_string();
        for id in [previous, leased, retired, &next] {
            fs::create_dir_all(install_root(&root, kind, id).unwrap()).unwrap();
        }
        write_selector(
            &root,
            kind,
            &Selector {
                current: Some(previous.into()),
                candidate: Some(next.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let legacy_lease = runtime(&root, kind, leased);
        let candidate = runtime(&root, kind, &next);
        confirm_ready(&candidate).unwrap();
        assert!(!install_root(&root, kind, retired).unwrap().exists());
        assert!(install_root(&root, kind, previous).unwrap().exists());
        assert!(install_root(&root, kind, leased).unwrap().exists());
        drop(legacy_lease);
        assert!(!install_root(&root, kind, leased).unwrap().exists());
        assert!(install_root(&root, kind, previous).unwrap().exists());
        drop(candidate);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selectors_reject_paths_and_staging_is_cleaned_when_cancelled() {
        let root = root();
        let kind = AgentKind::Codex;
        write_selector(
            &root,
            kind,
            &Selector {
                current: Some("../escape".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(read_selector(&root, kind).is_err());
        let path = root.join(".staging").join("discard");
        fs::create_dir_all(&path).unwrap();
        {
            let _guard = StagingCleanup {
                root: root.clone(),
                path: path.clone(),
            };
        }
        assert!(!path.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_record_requires_unpatched_schema_and_all_stored_policy_hashes() {
        let root = root();
        let kind = AgentKind::Codex;
        fs::write(root.join("package.json"), manifest(kind, "1.2.3").unwrap()).unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "test-lock").unwrap();
        fs::write(root.join("pnpm-workspace.yaml"), AGENT_WORKSPACE).unwrap();
        let mut record = InstallRecord {
            schema_version: 5,
            registry_id: provider(kind).registry_id.into(),
            adapter_name: provider(kind).adapter_name.into(),
            adapter_version: "1.2.3".into(),
            node_version: NODE_VERSION.into(),
            node_archive_sha256: NODE_ARCHIVE_SHA256.into(),
            pnpm_version: PNPM_VERSION.into(),
            pnpm_archive_sha512: PNPM_ARCHIVE_SHA512.into(),
            pnpm_lock_sha256: sha256_bytes(b"test-lock"),
            pnpm_workspace_sha256: sha256_bytes(AGENT_WORKSPACE),
            package_json_sha256: None,
        };
        let save = |record: &InstallRecord| {
            fs::write(
                root.join("lens-runtime.json"),
                serde_json::to_vec(record).unwrap(),
            )
            .unwrap()
        };
        save(&record);
        assert!(read_record(&root, kind).is_ok());
        record.schema_version = 4;
        save(&record);
        assert!(read_record(&root, kind).is_err());
        record.schema_version = 5;
        save(&record);
        fs::write(root.join("pnpm-lock.yaml"), "tampered").unwrap();
        assert!(read_record(&root, kind).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn archive_links_stay_within_the_managed_archive() {
        assert!(validate_archive_link(
            &Path::new(NODE_ARCHIVE_ROOT).join("bin/npm"),
            Path::new("../lib/node_modules/npm/bin/npm-cli.js"),
            NODE_ARCHIVE_ROOT
        )
        .is_ok());
        assert!(validate_archive_link(
            &Path::new(NODE_ARCHIVE_ROOT).join("bin/npm"),
            Path::new("../../escape"),
            NODE_ARCHIVE_ROOT
        )
        .is_err());
    }
    #[tokio::test]
    #[ignore = "requires LENS_UNPATCHED_CODEX_ROOT, LENS_UNPATCHED_CLAUDE_ROOT and LENS_UNPATCHED_NODE_ROOT"]
    async fn freshly_installed_unpatched_adapters_pass_runtime_verification() {
        let node = PathBuf::from(std::env::var_os("LENS_UNPATCHED_NODE_ROOT").unwrap());
        for (kind, variable) in [
            (AgentKind::Codex, "LENS_UNPATCHED_CODEX_ROOT"),
            (AgentKind::Claude, "LENS_UNPATCHED_CLAUDE_ROOT"),
        ] {
            let root =
                fs::canonicalize(PathBuf::from(std::env::var_os(variable).unwrap())).unwrap();
            assert!(
                root.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap())
                    || root.starts_with("/private/tmp")
            );
            assert!(root
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("lens-"));
            let json = package_json(&root).unwrap();
            let version = json["dependencies"][provider(kind).adapter_name]
                .as_str()
                .unwrap();
            verify_runtime_paths(&node, &root, kind, version)
                .await
                .unwrap();
            let migration = super::tests::root();
            let node_path = node_install_root(&migration);
            fs::create_dir_all(node_path.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&node, &node_path).unwrap();
            let legacy = provider_root(&migration, kind).join(version);
            copy_fixture(&root, &legacy);
            let policy = provider(kind);
            let record = InstallRecord {
                schema_version: 5,
                registry_id: policy.registry_id.into(),
                adapter_name: policy.adapter_name.into(),
                adapter_version: version.into(),
                node_version: NODE_VERSION.into(),
                node_archive_sha256: NODE_ARCHIVE_SHA256.into(),
                pnpm_version: PNPM_VERSION.into(),
                pnpm_archive_sha512: PNPM_ARCHIVE_SHA512.into(),
                pnpm_lock_sha256: sha256_file(&legacy.join("pnpm-lock.yaml"), "lock").unwrap(),
                pnpm_workspace_sha256: sha256_file(
                    &legacy.join("pnpm-workspace.yaml"),
                    "workspace",
                )
                .unwrap(),
                package_json_sha256: None,
            };
            fs::write(
                legacy.join("lens-runtime.json"),
                serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
            migrate_legacy(&migration, kind).await.unwrap();
            assert_eq!(
                read_selector(&migration, kind).unwrap().current,
                Some(format!("legacy:{version}"))
            );
            let restored = selected_runtime(&migration, kind).await.unwrap().unwrap();
            assert_eq!(restored.adapter_version, version);
            assert!(legacy.exists());
            drop(restored);
            fs::remove_dir_all(migration).unwrap();
        }
    }

    #[test]
    fn versions_and_registry_configuration_are_explicit() {
        for invalid in [
            "01.2.3",
            "1.2.3-",
            "1.2.3+",
            "1.2.3-01",
            "1.2.3+a..b",
            "latest",
        ] {
            assert!(!valid_version(invalid), "{invalid}");
        }
        for valid in ["1.2.3", "0.0.0", "1.2.3-rc.1+build.01"] {
            assert!(valid_version(valid), "{valid}");
        }
        assert!(version_order("1.10.0", "1.9.0").is_gt());
        assert!(version_order("1.10.0", "1.10.0-rc.1").is_gt());
        for extra in [
            serde_json::json!({"args":["--unsafe"]}),
            serde_json::json!({"env":{"NODE_OPTIONS":"--import=evil"}}),
        ] {
            let mut distribution =
                serde_json::json!({"package":"@agentclientprotocol/codex-acp@1.2.3"});
            distribution
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let registry = serde_json::json!({"version":"1.0.0","agents":[{"id":"codex-acp","version":"1.2.3","distribution":{"npx":distribution}}]});
            assert!(validate_registry_entry(
                &serde_json::to_vec(&registry).unwrap(),
                AgentKind::Codex
            )
            .is_err());
        }
    }
    #[test]
    fn ambient_package_manager_policy_overrides_are_removed_case_insensitively() {
        for key in [
            "npm_config_minimumReleaseAge",
            "NPM_CONFIG_IGNORE_SCRIPTS",
            "PnPm_PACKAGE_MANAGER_STRICT",
            "COREPACK_ENABLE_PROJECT_SPEC",
            "NODE_OPTIONS",
            "NODE_PATH",
        ] {
            assert!(package_manager_override(key));
        }
        for key in ["PATH", "TMPDIR", "HTTPS_PROXY"] {
            assert!(!package_manager_override(key));
        }
    }
    #[test]
    fn eligible_candidate_is_exact_bounded_and_preserves_the_entire_manifest() {
        let request = CandidateRequest::Eligible {
            ceiling: "1.11.0",
            after: None,
        };
        assert_eq!(request.selector().unwrap().as_deref(), Some("<=1.11.0"));
        let selected = manifest(AgentKind::Codex, "1.10.0").unwrap();
        assert_eq!(
            resolved_candidate_manifest(&selected, AgentKind::Codex, request)
                .unwrap()
                .0,
            "1.10.0"
        );
        for version in ["1.12.0", "^1.10.0", "latest", "npm:other@1.10.0"] {
            let mut value: serde_json::Value = serde_json::from_slice(&selected).unwrap();
            value["dependencies"][provider(AgentKind::Codex).adapter_name] = version.into();
            assert!(resolved_candidate_manifest(
                &serde_json::to_vec(&value).unwrap(),
                AgentKind::Codex,
                request
            )
            .is_err());
        }
        let mut modified: serde_json::Value = serde_json::from_slice(&selected).unwrap();
        modified["scripts"] = serde_json::json!({"install":"unexpected"});
        assert!(resolved_candidate_manifest(
            &serde_json::to_vec(&modified).unwrap(),
            AgentKind::Codex,
            request
        )
        .is_err());
        let update = CandidateRequest::Eligible {
            ceiling: "1.11.0",
            after: Some("1.10.0"),
        };
        assert_eq!(
            update.selector().unwrap().as_deref(),
            Some(">1.10.0 <=1.11.0")
        );
        assert!(!update.accepts("1.10.0"));
        assert!(!update.accepts("1.9.0"));
        assert!(update.accepts("1.11.0"));
        assert!(CandidateRequest::Eligible {
            ceiling: "1.9.0",
            after: Some("1.10.0")
        }
        .selector()
        .is_err());
    }

    #[tokio::test]
    #[ignore = "requires disposable LENS_DYNAMIC_ROOT and network; installs currently mature candidates without fixed version overrides"]
    async fn fresh_mature_install_uses_registry_ceiling_and_unchanged_safe_chain() {
        let root = fs::canonicalize(PathBuf::from(
            std::env::var_os("LENS_DYNAMIC_ROOT").unwrap(),
        ))
        .unwrap();
        assert!(
            root.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap())
                || root.starts_with("/private/tmp")
        );
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("lens-"));
        let app = tauri::test::mock_builder()
            .manage(crate::test_support::state())
            .build(crate::product_context())
            .unwrap();
        for kind in [AgentKind::Codex, AgentKind::Claude] {
            assert_eq!(read_selector(&root, kind).unwrap(), Selector::default());
            let ceiling = fetch_registry_version(kind).await.unwrap();
            let runtime = resolve_at(app.handle(), kind, true, &root).await.unwrap();
            assert!(CandidateRequest::Eligible {
                ceiling: &ceiling,
                after: None
            }
            .accepts(&runtime.adapter_version));
            let id = runtime.installation.as_ref().unwrap().id.clone();
            let path = install_root(&root, kind, &id).unwrap();
            let record = read_record(&path, kind).unwrap();
            assert_eq!(record.adapter_version, runtime.adapter_version);
            assert_eq!(
                fs::read(path.join("package.json")).unwrap(),
                manifest(kind, &runtime.adapter_version).unwrap()
            );
            assert_eq!(
                fs::read(path.join("pnpm-workspace.yaml")).unwrap(),
                AGENT_WORKSPACE
            );
            confirm_ready(&runtime).unwrap();
            let again = resolve_at(app.handle(), kind, true, &root).await.unwrap();
            assert_eq!(again.installation.as_ref().unwrap().id, id);
            assert!(read_selector(&root, kind).unwrap().candidate.is_none());
            eprintln!(
                "Mature resolution {:?}: registry {}, selected {}, repeat reused {}",
                kind, ceiling, runtime.adapter_version, id
            );
        }
    }

    fn expected_policy_rejection(error: &str) -> bool {
        [
            "ERR_PNPM_NO_MATURE_MATCHING_VERSION",
            "ERR_PNPM_TRUST_DOWNGRADE",
            "ERR_PNPM_MINIMUM_RELEASE_AGE",
        ]
        .iter()
        .any(|code| error.contains(code))
    }
    #[tokio::test]
    #[ignore = "requires disposable LENS_DYNAMIC_ROOT plus explicitly age-eligible LENS_DYNAMIC_CODEX_VERSION/LENS_DYNAMIC_CLAUDE_VERSION"]
    async fn fresh_dynamic_install_resolves_and_preserves_exact_inputs() {
        let root = fs::canonicalize(PathBuf::from(
            std::env::var_os("LENS_DYNAMIC_ROOT").unwrap(),
        ))
        .unwrap();
        assert!(
            root.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap())
                || root.starts_with("/private/tmp")
        );
        assert!(root
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("lens-"));
        let app = tauri::test::mock_builder()
            .manage(crate::test_support::state())
            .build(crate::product_context())
            .unwrap();
        let operation = Uuid::new_v4();
        let node = ensure_node_runtime(app.handle(), &root, operation)
            .await
            .unwrap();
        let pnpm = ensure_pnpm_runtime(app.handle(), &root, &node, operation)
            .await
            .unwrap();
        for (kind, env) in [
            (AgentKind::Codex, "LENS_DYNAMIC_CODEX_VERSION"),
            (AgentKind::Claude, "LENS_DYNAMIC_CLAUDE_VERSION"),
        ] {
            assert_eq!(read_selector(&root, kind).unwrap(), Selector::default());
            let latest = fetch_registry_version(kind).await.unwrap();
            let latest_result =
                install_candidate(app.handle(), &root, &node, &pnpm, kind, &latest, operation)
                    .await;
            let latest_blocked = match latest_result {
                Ok(runtime) => {
                    // Test-only state transition, not a claim of authenticated provider validation.
                    reject_candidate(&runtime).await.unwrap();
                    drop(runtime);
                    let _guard = selector_mutex().lock().unwrap();
                    write_selector(&root, kind, &Selector::default()).unwrap();
                    false
                }
                Err(error) => {
                    assert!(expected_policy_rejection(&error), "{error}");
                    assert_eq!(read_selector(&root, kind).unwrap(), Selector::default());
                    eprintln!("Official latest blocked by retained policy: {error}");
                    true
                }
            };
            // This explicit version override exists only in the opt-in acceptance fixture.
            let eligible = std::env::var(env).expect("explicit age-eligible test version");
            let runtime = install_candidate(
                app.handle(),
                &root,
                &node,
                &pnpm,
                kind,
                &eligible,
                operation,
            )
            .await
            .expect(
                "age-eligible exact fixture must install successfully through unchanged Safe-chain",
            );
            assert_eq!(runtime.adapter_version, eligible);
            let selector = read_selector(&root, kind).unwrap();
            assert!(selector.current.is_none());
            assert!(selector.candidate.is_some());
            let id = runtime.installation.as_ref().unwrap().id.clone();
            let record = read_record(&install_root(&root, kind, &id).unwrap(), kind).unwrap();
            assert_eq!(record.schema_version, AGENT_INSTALL_RECORD_VERSION);
            // Exercise the exact callback; actual initialize/session validation belongs to live tests.
            confirm_ready(&runtime).unwrap();
            let selected = resolve_at(app.handle(), kind, true, &root).await.unwrap();
            if latest_blocked {
                assert_eq!(selected.installation.as_ref().unwrap().id, id);
                let snapshot = app
                    .state::<AppState>()
                    .runtime
                    .read()
                    .unwrap()
                    .agent_runtime
                    .clone();
                assert_eq!(snapshot.stage, AgentRuntimeStage::Ready);
                assert!(snapshot
                    .error
                    .as_deref()
                    .is_some_and(expected_policy_rejection));
                assert_eq!(read_selector(&root, kind).unwrap().current, Some(id));
            }
        }
    }
}
