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
const NODE_VERSION: &str = "24.19.0";
const NODE_TARGET: &str = "darwin-arm64";
const NODE_ARCHIVE_NAME: &str = "node-v24.19.0-darwin-arm64.tar.gz";
const NODE_ARCHIVE_ROOT: &str = "node-v24.19.0-darwin-arm64";
const NODE_ARCHIVE_URL: &str = "https://nodejs.org/dist/v24.19.0/node-v24.19.0-darwin-arm64.tar.gz";
const NODE_ARCHIVE_SHA256: &str =
    "8294b7aa9b03997481c06babf1e8b270c859358f27da57a11509afe537ac381d";
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
const AGENT_INSTALL_RECORD_VERSION: u32 = 3;
const NODE_INSTALL_RECORD_VERSION: u32 = 2;
const PNPM_INSTALL_RECORD_VERSION: u32 = 1;
const NODE_TEAM_ID: &str = "HX7739G8FX";
const NODE_SIGNING_IDENTIFIER: &str = "node";
const CLAUDE_TEAM_ID: &str = "Q6L2SF6YDW";
const OPENAI_TEAM_ID: &str = "2DC432GLL2";

const CLAUDE_PACKAGE_JSON: &[u8] = include_bytes!("../agent-runtime/claude/package.json");
const CLAUDE_PNPM_LOCK: &[u8] = include_bytes!("../agent-runtime/claude/pnpm-lock.yaml");
const CLAUDE_PNPM_WORKSPACE: &[u8] = include_bytes!("../agent-runtime/claude/pnpm-workspace.yaml");
const CODEX_PACKAGE_JSON: &[u8] = include_bytes!("../agent-runtime/codex/package.json");
const CODEX_PNPM_LOCK: &[u8] = include_bytes!("../agent-runtime/codex/pnpm-lock.yaml");
const CODEX_PNPM_WORKSPACE: &[u8] = include_bytes!("../agent-runtime/codex/pnpm-workspace.yaml");

#[derive(Debug, Clone)]
pub struct ResolvedAgentRuntime {
    pub kind: AgentKind,
    pub adapter_name: &'static str,
    pub adapter_version: &'static str,
    pub safe_mode_id: &'static str,
    pub command: PathBuf,
    pub args: Vec<String>,
}

#[derive(Clone, Copy)]
struct SignedExecutablePolicy {
    relative_path: &'static str,
    team_id: &'static str,
    signing_identifier: &'static str,
    label: &'static str,
}

#[derive(Clone, Copy)]
struct AgentRuntimePolicy {
    kind: AgentKind,
    registry_id: &'static str,
    adapter_name: &'static str,
    adapter_version: &'static str,
    adapter_version_output: &'static str,
    safe_mode_id: &'static str,
    package_json: &'static [u8],
    pnpm_lock: &'static [u8],
    pnpm_workspace: &'static [u8],
    entrypoint: &'static str,
    signed_executables: &'static [SignedExecutablePolicy],
}

const CLAUDE_SIGNED_EXECUTABLES: &[SignedExecutablePolicy] = &[SignedExecutablePolicy {
    relative_path: "node_modules/.pnpm/@anthropic-ai+claude-agent-sdk-darwin-arm64@0.3.232/node_modules/@anthropic-ai/claude-agent-sdk-darwin-arm64/claude",
    team_id: CLAUDE_TEAM_ID,
    signing_identifier: "com.anthropic.claude-code",
    label: "Claude runtime",
}];

const CODEX_SIGNED_EXECUTABLES: &[SignedExecutablePolicy] = &[
    SignedExecutablePolicy {
        relative_path: "node_modules/.pnpm/@openai+codex@0.148.0-darwin-arm64/node_modules/@openai/codex/vendor/aarch64-apple-darwin/bin/codex",
        team_id: OPENAI_TEAM_ID,
        signing_identifier: "codex",
        label: "Codex runtime",
    },
    SignedExecutablePolicy {
        relative_path: "node_modules/.pnpm/@openai+codex@0.148.0-darwin-arm64/node_modules/@openai/codex/vendor/aarch64-apple-darwin/bin/codex-code-mode-host",
        team_id: OPENAI_TEAM_ID,
        signing_identifier: "codex-code-mode-host",
        label: "Codex code-mode host",
    },
];

fn policy(kind: AgentKind) -> AgentRuntimePolicy {
    match kind {
        AgentKind::Claude => AgentRuntimePolicy {
            kind,
            registry_id: "claude-acp",
            adapter_name: "@agentclientprotocol/claude-agent-acp",
            adapter_version: "0.70.0",
            adapter_version_output: "0.70.0",
            safe_mode_id: "plan",
            package_json: CLAUDE_PACKAGE_JSON,
            pnpm_lock: CLAUDE_PNPM_LOCK,
            pnpm_workspace: CLAUDE_PNPM_WORKSPACE,
            entrypoint: "node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js",
            signed_executables: CLAUDE_SIGNED_EXECUTABLES,
        },
        AgentKind::Codex => AgentRuntimePolicy {
            kind,
            registry_id: "codex-acp",
            adapter_name: "@agentclientprotocol/codex-acp",
            adapter_version: "1.6.2",
            adapter_version_output: "@agentclientprotocol/codex-acp 1.6.2",
            safe_mode_id: "read-only",
            package_json: CODEX_PACKAGE_JSON,
            pnpm_lock: CODEX_PNPM_LOCK,
            pnpm_workspace: CODEX_PNPM_WORKSPACE,
            entrypoint: "node_modules/@agentclientprotocol/codex-acp/dist/index.js",
            signed_executables: CODEX_SIGNED_EXECUTABLES,
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
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
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

pub async fn resolve(app: &AppHandle, kind: AgentKind) -> Result<ResolvedAgentRuntime, String> {
    let state = app.state::<AppState>();
    let _install_guard = state.agent_runtime_install.lock().await;
    let operation_id = Uuid::new_v4();
    publish_agent_runtime(
        app,
        runtime_state(
            operation_id,
            kind,
            AgentRuntimeStage::Resolving,
            "Resolving approved Agent runtime…",
        ),
    )?;

    let result = resolve_inner(app, kind, operation_id, true).await;
    if let Err(error) = &result {
        update_agent_runtime(app, operation_id, |runtime| {
            runtime.stage = AgentRuntimeStage::Failed;
            runtime.message = None;
            runtime.error = Some(error.clone());
            runtime.total_bytes = None;
        })?;
    }
    result
}

pub async fn resolve_installed(
    app: &AppHandle,
    kind: AgentKind,
) -> Result<Option<ResolvedAgentRuntime>, String> {
    let state = app.state::<AppState>();
    let _install_guard = state.agent_runtime_install.lock().await;
    let operation_id = Uuid::new_v4();
    publish_agent_runtime(
        app,
        runtime_state(
            operation_id,
            kind,
            AgentRuntimeStage::Resolving,
            "Checking the installed Agent runtime…",
        ),
    )?;

    match resolve_inner(app, kind, operation_id, false).await {
        Ok(runtime) => Ok(Some(runtime)),
        Err(error) if error == not_installed_error(kind) => {
            update_agent_runtime(app, operation_id, |runtime| {
                runtime.stage = AgentRuntimeStage::NotInstalled;
                runtime.message = Some(format!(
                    "{} will be downloaded when selected.",
                    display_name(kind)
                ));
                runtime.error = None;
            })?;
            Ok(None)
        }
        Err(error) => {
            update_agent_runtime(app, operation_id, |runtime| {
                runtime.stage = AgentRuntimeStage::Failed;
                runtime.message = None;
                runtime.error = Some(error.clone());
            })?;
            Err(error)
        }
    }
}

async fn resolve_inner(
    app: &AppHandle,
    kind: AgentKind,
    operation_id: Uuid,
    install_if_missing: bool,
) -> Result<ResolvedAgentRuntime, String> {
    ensure_supported_target()?;
    let approved = policy(kind);
    let root = runtime_root(app)?;

    match verify_installed_runtime(&root, approved, operation_id, app).await {
        Ok(runtime) => {
            publish_ready(app, operation_id, approved)?;
            return Ok(runtime);
        }
        Err(error) if !install_if_missing && error != not_installed_error(kind) => {
            return Err(error);
        }
        Err(_) if !install_if_missing => return Err(not_installed_error(kind)),
        Err(_) => {}
    }

    update_agent_runtime(app, operation_id, |runtime| {
        runtime.stage = AgentRuntimeStage::Resolving;
        runtime.message = Some("Checking the official ACP Registry entry…".into());
        runtime.error = None;
    })?;
    verify_registry_entry(approved).await?;

    fs::create_dir_all(&root)
        .map_err(|error| format!("unable to create managed runtime directory: {error}"))?;
    let node_root = ensure_node_runtime(app, &root, operation_id).await?;
    let pnpm_root = ensure_pnpm_runtime(app, &root, &node_root, operation_id).await?;
    let agent_root =
        ensure_agent_runtime(app, &root, &node_root, &pnpm_root, approved, operation_id).await?;
    verify_install_record(&agent_root, approved)?;
    let runtime = verify_runtime_paths(&node_root, &agent_root, approved).await?;
    publish_ready(app, operation_id, approved)?;
    Ok(runtime)
}

fn runtime_state(
    operation_id: Uuid,
    kind: AgentKind,
    stage: AgentRuntimeStage,
    message: &str,
) -> AgentRuntimeState {
    AgentRuntimeState {
        operation_id: Some(operation_id),
        stage,
        agent: Some(kind),
        version: Some(policy(kind).adapter_version.into()),
        downloaded_bytes: 0,
        total_bytes: None,
        message: Some(message.into()),
        error: None,
    }
}

fn publish_ready(
    app: &AppHandle,
    operation_id: Uuid,
    approved: AgentRuntimePolicy,
) -> Result<(), String> {
    update_agent_runtime(app, operation_id, |runtime| {
        runtime.stage = AgentRuntimeStage::Ready;
        runtime.version = Some(approved.adapter_version.into());
        runtime.downloaded_bytes = 0;
        runtime.total_bytes = None;
        runtime.message = Some(format!(
            "{} {} is installed and verified.",
            display_name(approved.kind),
            approved.adapter_version
        ));
        runtime.error = None;
    })?;
    Ok(())
}

fn runtime_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_local_data_dir()
        .map(|path| path.join("agent-runtimes"))
        .map_err(|error| format!("unable to resolve application data directory: {error}"))
}

fn node_install_root(root: &Path) -> PathBuf {
    root.join("node")
        .join(format!("v{NODE_VERSION}-{NODE_TARGET}"))
}

fn pnpm_install_root(root: &Path) -> PathBuf {
    root.join("pnpm").join(format!("v{PNPM_VERSION}"))
}

fn agent_install_root(root: &Path, approved: AgentRuntimePolicy) -> PathBuf {
    root.join("agents")
        .join(approved.registry_id)
        .join(approved.adapter_version)
}

async fn verify_installed_runtime(
    root: &Path,
    approved: AgentRuntimePolicy,
    operation_id: Uuid,
    app: &AppHandle,
) -> Result<ResolvedAgentRuntime, String> {
    let node_root = node_install_root(root);
    let agent_root = agent_install_root(root, approved);
    if !node_root.is_dir() || !agent_root.is_dir() {
        return Err(not_installed_error(approved.kind));
    }

    update_agent_runtime(app, operation_id, |runtime| {
        runtime.stage = AgentRuntimeStage::Verifying;
        runtime.message = Some("Verifying the installed Agent runtime…".into());
    })?;
    verify_install_record(&agent_root, approved)?;
    verify_runtime_paths(&node_root, &agent_root, approved).await
}

fn verify_install_record(agent_root: &Path, approved: AgentRuntimePolicy) -> Result<(), String> {
    let record_path = agent_root.join("lens-runtime.json");
    let record_bytes = fs::read(&record_path)
        .map_err(|error| format!("managed Agent install record is unavailable: {error}"))?;
    let record = serde_json::from_slice::<InstallRecord>(&record_bytes)
        .map_err(|error| format!("managed Agent install record is invalid: {error}"))?;
    if record.schema_version != AGENT_INSTALL_RECORD_VERSION
        || record.registry_id != approved.registry_id
        || record.adapter_name != approved.adapter_name
        || record.adapter_version != approved.adapter_version
        || record.node_version != NODE_VERSION
        || record.node_archive_sha256 != NODE_ARCHIVE_SHA256
        || record.pnpm_version != PNPM_VERSION
        || record.pnpm_archive_sha512 != PNPM_ARCHIVE_SHA512
        || record.pnpm_lock_sha256 != sha256_bytes(approved.pnpm_lock)
        || record.pnpm_workspace_sha256 != sha256_bytes(approved.pnpm_workspace)
    {
        return Err("managed Agent install record does not match the approved policy".into());
    }
    if fs::read(agent_root.join("package.json")).ok().as_deref() != Some(approved.package_json)
        || fs::read(agent_root.join("pnpm-lock.yaml")).ok().as_deref() != Some(approved.pnpm_lock)
        || fs::read(agent_root.join("pnpm-workspace.yaml"))
            .ok()
            .as_deref()
            != Some(approved.pnpm_workspace)
    {
        return Err("managed Agent package policy files were modified".into());
    }
    Ok(())
}

async fn verify_runtime_paths(
    node_root: &Path,
    agent_root: &Path,
    approved: AgentRuntimePolicy,
) -> Result<ResolvedAgentRuntime, String> {
    verify_node_runtime(node_root).await?;
    let node = canonical_managed_file(node_root, &node_root.join("bin/node"), "Node runtime")?;
    let entrypoint = canonical_managed_file(
        agent_root,
        &agent_root.join(approved.entrypoint),
        "ACP adapter",
    )?;

    for executable in approved.signed_executables {
        let path = canonical_managed_file(
            agent_root,
            &agent_root.join(executable.relative_path),
            executable.label,
        )?;
        verify_code_signature(
            &path,
            executable.team_id,
            executable.signing_identifier,
            executable.label,
        )
        .await?;
    }

    verify_version_command(
        &node,
        &[entrypoint.to_string_lossy().as_ref(), "--version"],
        approved.adapter_version_output,
        "ACP adapter",
    )
    .await?;

    Ok(ResolvedAgentRuntime {
        kind: approved.kind,
        adapter_name: approved.adapter_name,
        adapter_version: approved.adapter_version,
        safe_mode_id: approved.safe_mode_id,
        command: node,
        args: vec![entrypoint.to_string_lossy().into_owned()],
    })
}

async fn verify_registry_entry(approved: AgentRuntimePolicy) -> Result<(), String> {
    let client = http_client()?;
    let mut response =
        tokio::time::timeout(Duration::from_secs(30), client.get(REGISTRY_URL).send())
            .await
            .map_err(|_| "official ACP Registry request timed out".to_string())?
            .map_err(|error| format!("unable to fetch official ACP Registry: {error}"))?
            .error_for_status()
            .map_err(|error| format!("official ACP Registry returned an error: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > REGISTRY_MAX_BYTES as u64)
    {
        return Err("official ACP Registry response exceeds the supported size".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("unable to read official ACP Registry: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > REGISTRY_MAX_BYTES {
            return Err("official ACP Registry response exceeds the supported size".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    validate_registry_entry(&bytes, approved)
}

fn validate_registry_entry(bytes: &[u8], approved: AgentRuntimePolicy) -> Result<(), String> {
    if bytes.len() > REGISTRY_MAX_BYTES {
        return Err("official ACP Registry response exceeds the supported size".into());
    }
    let registry = serde_json::from_slice::<RegistryIndex>(bytes)
        .map_err(|error| format!("official ACP Registry is invalid: {error}"))?;
    if registry.version != REGISTRY_SCHEMA_VERSION {
        return Err(format!(
            "unsupported ACP Registry schema version {}",
            registry.version
        ));
    }
    let mut matches = registry
        .agents
        .into_iter()
        .filter(|agent| agent.id == approved.registry_id);
    let entry = matches.next().ok_or_else(|| {
        format!(
            "{} is absent from the official ACP Registry",
            approved.registry_id
        )
    })?;
    if matches.next().is_some() {
        return Err(format!(
            "{} appears more than once in the official ACP Registry",
            approved.registry_id
        ));
    }
    let npx = entry.distribution.npx.ok_or_else(|| {
        format!(
            "{} has no npm distribution in the official ACP Registry",
            approved.registry_id
        )
    })?;
    let expected_registry_spec = format!("{}@{}", approved.adapter_name, entry.version);
    if npx.package != expected_registry_spec {
        return Err(format!(
            "official ACP Registry package {} does not match {}",
            npx.package, expected_registry_spec
        ));
    }
    Ok(())
}

async fn ensure_node_runtime(
    app: &AppHandle,
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

async fn download_node_archive(
    app: &AppHandle,
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

async fn ensure_pnpm_runtime(
    app: &AppHandle,
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
        &[pnpm_cli.to_string_lossy().as_ref(), "--version"],
        PNPM_VERSION,
        "pnpm",
    )
    .await
}

async fn download_pnpm_archive(
    app: &AppHandle,
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

async fn ensure_agent_runtime(
    app: &AppHandle,
    root: &Path,
    node_root: &Path,
    pnpm_root: &Path,
    approved: AgentRuntimePolicy,
    operation_id: Uuid,
) -> Result<PathBuf, String> {
    let final_root = agent_install_root(root, approved);
    if final_root.is_dir() {
        match verify_install_record(&final_root, approved) {
            Ok(())
                if verify_runtime_paths(node_root, &final_root, approved)
                    .await
                    .is_ok() =>
            {
                return Ok(final_root)
            }
            _ => quarantine_existing(root, &final_root, approved.registry_id)?,
        }
    }

    update_agent_runtime(app, operation_id, |runtime| {
        runtime.stage = AgentRuntimeStage::Installing;
        runtime.message = Some(format!(
            "Installing {} {} from its approved pnpm dependency lock through Takumi Guard…",
            display_name(approved.kind),
            approved.adapter_version
        ));
        runtime.downloaded_bytes = 0;
        runtime.total_bytes = None;
    })?;

    let staging_root = root.join(".staging").join(Uuid::new_v4().to_string());
    let staged_agent = staging_root.join("agent");
    let pnpm_store = staging_root.join("pnpm-store");
    fs::create_dir_all(&staged_agent)
        .map_err(|error| format!("unable to create Agent staging directory: {error}"))?;
    fs::write(staged_agent.join("package.json"), approved.package_json)
        .map_err(|error| format!("unable to write Agent package policy: {error}"))?;
    fs::write(staged_agent.join("pnpm-lock.yaml"), approved.pnpm_lock)
        .map_err(|error| format!("unable to write Agent dependency lock: {error}"))?;
    fs::write(
        staged_agent.join("pnpm-workspace.yaml"),
        approved.pnpm_workspace,
    )
    .map_err(|error| format!("unable to write Agent pnpm policy: {error}"))?;
    fs::write(staging_root.join("blank-user-npmrc"), [])
        .map_err(|error| format!("unable to create pnpm policy file: {error}"))?;
    fs::write(staging_root.join("blank-global-npmrc"), [])
        .map_err(|error| format!("unable to create pnpm policy file: {error}"))?;

    let result = async {
        run_pnpm_install(
            node_root,
            pnpm_root,
            &staged_agent,
            &pnpm_store,
            &staging_root,
        )
        .await?;
        verify_runtime_paths(node_root, &staged_agent, approved).await?;
        let record = serde_json::to_vec_pretty(&install_record(approved))
            .map_err(|error| format!("unable to serialize Agent install record: {error}"))?;
        fs::write(staged_agent.join("lens-runtime.json"), record)
            .map_err(|error| format!("unable to write Agent install record: {error}"))?;
        let parent = final_root
            .parent()
            .ok_or_else(|| "managed Agent path has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("unable to create managed Agent directory: {error}"))?;
        match fs::rename(&staged_agent, &final_root) {
            Ok(()) => Ok(final_root.clone()),
            Err(_error) if final_root.is_dir() => {
                verify_install_record(&final_root, approved)?;
                Ok(final_root.clone())
            }
            Err(error) => Err(format!("unable to activate managed Agent runtime: {error}")),
        }
    }
    .await;
    cleanup_staging(root, &staging_root);
    result
}

async fn run_pnpm_install(
    node_root: &Path,
    pnpm_root: &Path,
    agent_root: &Path,
    pnpm_store: &Path,
    staging_root: &Path,
) -> Result<(), String> {
    let node = canonical_managed_file(node_root, &node_root.join("bin/node"), "Node runtime")?;
    let pnpm_cli = canonical_managed_file(pnpm_root, &pnpm_root.join("bin/pnpm.mjs"), "pnpm CLI")?;
    let user_config = staging_root.join("blank-user-npmrc");
    let global_config = staging_root.join("blank-global-npmrc");
    let future = Command::new(&node)
        .arg(&pnpm_cli)
        .args([
            "install",
            "--prod",
            "--frozen-lockfile",
            "--registry=https://npm.flatt.tech/",
        ])
        .arg(format!("--store-dir={}", pnpm_store.display()))
        .current_dir(agent_root)
        .env("NODE_OPTIONS", "")
        .env("NODE_PATH", "")
        .env("NODE_TLS_REJECT_UNAUTHORIZED", "1")
        .env("PNPM_HOME", "")
        .env("COREPACK_HOME", "")
        .env("NPM_CONFIG_USERCONFIG", &user_config)
        .env("NPM_CONFIG_GLOBALCONFIG", &global_config)
        .env("npm_config_node_options", "")
        .env("npm_config_update_notifier", "false")
        .env("npm_config_registry", TAKUMI_GUARD_REGISTRY)
        .stdin(Stdio::null())
        .output();
    let output = tokio::time::timeout(Duration::from_secs(600), future)
        .await
        .map_err(|_| "managed Agent pnpm installation timed out".to_string())?
        .map_err(|error| format!("unable to start managed Agent pnpm installation: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "managed Agent pnpm installation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn install_record(approved: AgentRuntimePolicy) -> InstallRecord {
    InstallRecord {
        schema_version: AGENT_INSTALL_RECORD_VERSION,
        registry_id: approved.registry_id.into(),
        adapter_name: approved.adapter_name.into(),
        adapter_version: approved.adapter_version.into(),
        node_version: NODE_VERSION.into(),
        node_archive_sha256: NODE_ARCHIVE_SHA256.into(),
        pnpm_version: PNPM_VERSION.into(),
        pnpm_archive_sha512: PNPM_ARCHIVE_SHA512.into(),
        pnpm_lock_sha256: sha256_bytes(approved.pnpm_lock),
        pnpm_workspace_sha256: sha256_bytes(approved.pnpm_workspace),
    }
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
    let future = Command::new(command)
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

fn not_installed_error(kind: AgentKind) -> String {
    format!("{} managed runtime is not installed", display_name(kind))
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

    fn registry(agent: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": "1.0.0",
            "agents": [agent]
        }))
        .unwrap()
    }

    #[test]
    fn approved_registry_identity_accepts_a_newer_version_of_the_same_package() {
        let bytes = registry(serde_json::json!({
            "id": "codex-acp",
            "version": "1.7.0",
            "distribution": {
                "npx": { "package": "@agentclientprotocol/codex-acp@1.7.0" }
            }
        }));
        assert!(validate_registry_entry(&bytes, policy(AgentKind::Codex)).is_ok());
    }

    #[test]
    fn registry_package_identity_must_match_the_approved_adapter() {
        let bytes = registry(serde_json::json!({
            "id": "codex-acp",
            "version": "1.6.2",
            "distribution": {
                "npx": { "package": "@example/impersonator@1.6.2" }
            }
        }));
        assert!(validate_registry_entry(&bytes, policy(AgentKind::Codex)).is_err());
    }

    #[test]
    fn archive_paths_and_links_are_contained_by_the_approved_root() {
        assert!(validate_archive_path(
            Path::new("node-v24.19.0-darwin-arm64/bin/node"),
            NODE_ARCHIVE_ROOT
        )
        .is_ok());
        assert!(validate_archive_path(Path::new("../escape"), NODE_ARCHIVE_ROOT).is_err());
        assert!(validate_archive_link(
            Path::new("node-v24.19.0-darwin-arm64/bin/npm"),
            Path::new("../lib/node_modules/npm/bin/npm-cli.js"),
            NODE_ARCHIVE_ROOT
        )
        .is_ok());
        assert!(validate_archive_link(
            Path::new("node-v24.19.0-darwin-arm64/bin/npm"),
            Path::new("../../escape"),
            NODE_ARCHIVE_ROOT
        )
        .is_err());
    }

    #[test]
    fn install_record_is_derived_from_the_embedded_dependency_lock() {
        let approved = policy(AgentKind::Claude);
        let record = install_record(approved);
        assert_eq!(record.schema_version, AGENT_INSTALL_RECORD_VERSION);
        assert_eq!(record.adapter_version, "0.70.0");
        assert_eq!(record.pnpm_lock_sha256, sha256_bytes(CLAUDE_PNPM_LOCK));
        assert_eq!(record.pnpm_version, "11.22.0");
        assert_eq!(
            record.pnpm_workspace_sha256,
            sha256_bytes(CLAUDE_PNPM_WORKSPACE)
        );
    }
}
