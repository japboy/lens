//! Google-signed binary distribution. Shared selectors own activation and leases.
use super::*;
use chrono::DateTime;
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, SeekFrom, Write},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use zip::{read::ArchiveOffset, read::Config as ZipReadConfig, ZipArchive};

const TARGET: &str = "darwin-aarch64";
const GOOGLE_TEAM_ID: &str = "EQHXZ8M8AV";
const EXECUTABLES: [(&str, &str); 2] = [
    ("agy_acp_server.par", "agy_acp_server"),
    ("localharness_external", "localharness_external"),
];
const RECORD: &str = "lens-binary-runtime.json";
const RECORD_VERSION: u32 = 1;
const ARCHIVE_MAX_BYTES: u64 = 256 * 1024 * 1024;
const EXECUTABLE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const PAYLOAD_MAX_BYTES: u64 = 768 * 1024 * 1024;
const MIN_ARCHIVE_AGE_SECONDS: i64 = 60 * 60;
const IDENTITY_MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RegistryBinaryDistribution {
    archive: String,
    cmd: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub(super) struct Release {
    pub version: String,
    archive_url: String,
    sha256: Option<String>,
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn archive_url(version: &str) -> Result<String, String> {
    if !valid_version(version) {
        return Err("invalid official Antigravity version".into());
    }
    Ok(format!("https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-{version}-darwin-arm64.zip"))
}

pub(super) fn validate_distribution(
    version: &str,
    distribution: &RegistryDistribution,
) -> Result<Release, String> {
    let value = distribution
        .binary
        .as_ref()
        .and_then(|binaries| binaries.as_object())
        .and_then(|binaries| binaries.get(TARGET))
        .ok_or("Antigravity has no supported official binary distribution")?;
    let binary: RegistryBinaryDistribution = serde_json::from_value(value.clone())
        .map_err(|_| "official Antigravity binary distribution has an unsupported shape")?;
    if distribution.npx.is_some()
        || binary.archive != archive_url(version)?
        || binary.cmd != "./agy_acp_server.par"
        || !binary.args.is_empty()
        || !binary.env.is_empty()
        || binary
            .sha256
            .as_deref()
            .is_some_and(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("official Antigravity binary distribution identity mismatch".into());
    }
    Ok(Release {
        version: version.to_owned(),
        archive_url: binary.archive.clone(),
        sha256: binary.sha256.as_ref().map(|hash| hash.to_ascii_lowercase()),
    })
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BinaryRecord {
    schema_version: u32,
    distribution: String,
    registry_id: String,
    adapter_version: String,
    target: String,
    archive_url: String,
    archive_sha256: String,
    registry_sha256: Option<String>,
    archive_bytes: u64,
    archive_last_modified: String,
    observed_at_unix_seconds: i64,
    files: BTreeMap<String, BinaryFile>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BinaryFile {
    sha256: String,
    bytes: u64,
}

fn now_seconds() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock predates the UNIX epoch".to_owned())?
        .as_secs()
        .try_into()
        .map_err(|_| "system clock is out of range".into())
}

/// Google HTTPS file modification time is artifact age, not a publication date.
fn archive_is_mature(last_modified: &str, observed: i64) -> Result<bool, String> {
    let modified = DateTime::parse_from_rfc2822(last_modified)
        .map_err(|_| "official Antigravity archive Last-Modified is invalid".to_owned())?
        .timestamp();
    if modified < 0 || observed < modified {
        return Err(
            "official Antigravity archive Last-Modified is in the future or out of range".into(),
        );
    }
    Ok(observed - modified >= MIN_ARCHIVE_AGE_SECONDS)
}

struct Download {
    sha256: String,
    bytes: u64,
    last_modified: String,
    observed: i64,
}

async fn download<R: tauri::Runtime>(
    app: &AppHandle<R>,
    operation: Uuid,
    release: &Release,
    destination: &Path,
) -> Result<Option<Download>, String> {
    let expected_url = &release.archive_url;
    let mut response = http_client_with_redirects(reqwest::redirect::Policy::none())?
        .get(expected_url)
        .send()
        .await
        .map_err(|error| format!("unable to download official Antigravity archive: {error}"))?
        .error_for_status()
        .map_err(|error| format!("official Antigravity archive download failed: {error}"))?;
    if response.status() != reqwest::StatusCode::OK
        || response.url().as_str() != expected_url.as_str()
    {
        return Err("official Antigravity archive response changed its origin or status".into());
    }
    let observed = now_seconds()?;
    let headers = response.headers().get_all(reqwest::header::LAST_MODIFIED);
    let mut values = headers.iter();
    let last_modified = values
        .next()
        .ok_or("official Antigravity archive has no Last-Modified; archive age cannot be verified")?
        .to_str()
        .map_err(|_| "official Antigravity archive Last-Modified is invalid")?
        .to_owned();
    if values.next().is_some() {
        return Err("official Antigravity archive has ambiguous Last-Modified metadata".into());
    }
    if !archive_is_mature(&last_modified, observed)? {
        return Ok(None);
    }
    let total = response.content_length();
    if total.is_some_and(|bytes| bytes == 0 || bytes > ARCHIVE_MAX_BYTES) {
        return Err("official Antigravity archive exceeds the download size policy".into());
    }
    update_agent_runtime(app, operation, |state| state.total_bytes = total)?;
    // Synchronous open prevents a deferred open racing cancellation cleanup.
    let file = File::create(destination).map_err(|error| error.to_string())?;
    let mut file = tokio::fs::File::from_std(file);
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut published = 0_u64;
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        bytes = bytes
            .checked_add(chunk.len() as u64)
            .ok_or("Antigravity archive size overflow")?;
        if bytes > ARCHIVE_MAX_BYTES {
            return Err("official Antigravity archive exceeds the download size policy".into());
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| error.to_string())?;
        hash.update(&chunk);
        if bytes.saturating_sub(published) >= 1024 * 1024 {
            update_agent_runtime(app, operation, |state| state.downloaded_bytes = bytes)?;
            published = bytes;
        }
    }
    file.flush().await.map_err(|error| error.to_string())?;
    if bytes == 0 || total.is_some_and(|total| total != bytes) {
        return Err("official Antigravity archive download is incomplete".into());
    }
    update_agent_runtime(app, operation, |state| state.downloaded_bytes = bytes)?;
    let sha256 = hex_digest(&hash.finalize());
    if release
        .sha256
        .as_deref()
        .is_some_and(|expected| expected != sha256)
    {
        return Err("official Antigravity archive does not match its Registry SHA-256".into());
    }
    Ok(Some(Download {
        sha256,
        bytes,
        last_modified,
        observed,
    }))
}

fn allowed_name(name: &str) -> bool {
    EXECUTABLES.iter().any(|(expected, _)| *expected == name)
}
/// Bound metadata allocation before handing the archive to zip-rs. The official
/// payload uses a single-disk ZIP32 with two entries and no archive comment.
fn validate_zip_envelope(file: &mut File) -> Result<(u64, Vec<u8>, u64), String> {
    let size = file.metadata().map_err(|error| error.to_string())?.len();
    if !(22..=ARCHIVE_MAX_BYTES).contains(&size) {
        return Err("Antigravity ZIP exceeds the archive size policy".into());
    }
    file.seek(SeekFrom::End(-22))
        .map_err(|error| error.to_string())?;
    let mut end = [0_u8; 22];
    file.read_exact(&mut end)
        .map_err(|error| error.to_string())?;
    let word = |at: usize| u16::from_le_bytes([end[at], end[at + 1]]);
    let dword =
        |at: usize| u32::from_le_bytes([end[at], end[at + 1], end[at + 2], end[at + 3]]) as u64;
    if end[..4] != [0x50, 0x4b, 0x05, 0x06]
        || word(4) != 0
        || word(6) != 0
        || word(8) != 2
        || word(10) != 2
        || word(20) != 0
        || !(92..=128 * 1024).contains(&dword(12))
        || dword(16).checked_add(dword(12)) != Some(size - 22)
    {
        return Err("Antigravity ZIP must have exactly two entries in a bounded single-disk ZIP32 directory".into());
    }
    let metadata_start = dword(16);
    file.seek(SeekFrom::Start(metadata_start))
        .map_err(|error| error.to_string())?;
    let mut metadata = vec![0_u8; (size - metadata_start) as usize];
    file.read_exact(&mut metadata)
        .map_err(|error| error.to_string())?;
    // zip-rs can retry an earlier EOCD when the final directory is malformed.
    // Reject alternate EOCDs in this bounded window; the reader below hides the
    // payload during metadata parsing, so it cannot discover one there either.
    if metadata[..metadata.len() - 22]
        .windows(4)
        .any(|bytes| bytes == b"PK\x05\x06")
    {
        return Err("Antigravity ZIP contains ambiguous directory terminators".into());
    }
    Ok((metadata_start, metadata, size))
}

struct ZipPolicyReader<'a> {
    file: File,
    size: u64,
    position: u64,
    metadata_start: u64,
    metadata: &'a [u8],
    metadata_only: &'a Cell<bool>,
}
impl Read for ZipPolicyReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let length = buffer.len().min((self.size - self.position) as usize);
        let read = if self.metadata_only.get() {
            buffer[..length].fill(0);
            let start = self.position.max(self.metadata_start);
            let end = self.position + length as u64;
            if start < end {
                let source = (start - self.metadata_start) as usize;
                let destination = (start - self.position) as usize;
                let count = (end - start) as usize;
                buffer[destination..destination + count]
                    .copy_from_slice(&self.metadata[source..source + count]);
            }
            length
        } else {
            self.file.seek(SeekFrom::Start(self.position))?;
            self.file.read(&mut buffer[..length])?
        };
        self.position += read as u64;
        Ok(read)
    }
}
impl Seek for ZipPolicyReader<'_> {
    fn seek(&mut self, requested: SeekFrom) -> std::io::Result<u64> {
        let position = match requested {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => i128::from(self.size) + i128::from(offset),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
        };
        if position < 0 || position > i128::from(self.size) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "ZIP seek exceeds its verified envelope",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

/// No archive-controlled path or permission is used to create output files.
fn extract(archive: &Path, destination: &Path) -> Result<(), String> {
    let mut file = File::open(archive).map_err(|error| error.to_string())?;
    let (metadata_start, metadata, size) = validate_zip_envelope(&mut file)?;
    let metadata_only = Cell::new(true);
    let reader = ZipPolicyReader {
        file,
        size,
        position: 0,
        metadata_start,
        metadata: &metadata,
        metadata_only: &metadata_only,
    };
    let mut archive = ZipArchive::with_config(
        ZipReadConfig {
            archive_offset: ArchiveOffset::Known(0),
        },
        reader,
    )
    .map_err(|error| format!("invalid official Antigravity ZIP: {error}"))?;
    metadata_only.set(false);
    if archive.len() != EXECUTABLES.len() {
        return Err("Antigravity ZIP has missing or duplicate entries".into());
    }
    let mut names = BTreeSet::new();
    let mut total = 0_u64;
    // Validate the complete metadata before creating any payload file.
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|error| error.to_string())?;
        if !allowed_name(file.name())
            || file.name_raw() != file.name().as_bytes()
            || !file.is_file()
            || !names.insert(file.name().to_owned())
            || !file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o100000)
            || file.size() == 0
            || file.size() > EXECUTABLE_MAX_BYTES
            || file.compressed_size() > ARCHIVE_MAX_BYTES
        {
            return Err(
                "Antigravity ZIP contains an unexpected, oversized or non-regular entry".into(),
            );
        }
        total = total
            .checked_add(file.size())
            .ok_or("Antigravity ZIP size overflow")?;
        if total > PAYLOAD_MAX_BYTES {
            return Err("Antigravity ZIP exceeds the expanded size policy".into());
        }
    }
    fs::create_dir(destination).map_err(|error| error.to_string())?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| error.to_string())?;
        let expected_size = file.size();
        let path = destination.join(file.name());
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| error.to_string())?;
        let actual = std::io::copy(
            &mut Read::take(&mut file, EXECUTABLE_MAX_BYTES + 1),
            &mut output,
        )
        .map_err(|error| format!("Antigravity ZIP entry integrity check failed: {error}"))?;
        if actual != expected_size || actual > EXECUTABLE_MAX_BYTES {
            return Err("Antigravity ZIP entry length is inconsistent".into());
        }
        output.flush().map_err(|error| error.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn file_identity(path: &Path) -> Result<BinaryFile, String> {
    let mut input = File::open(path).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = input.read(&mut buffer).map_err(|error| error.to_string())?;
        if length == 0 {
            break;
        }
        bytes += length as u64;
        if bytes > EXECUTABLE_MAX_BYTES {
            return Err("Antigravity executable exceeds size policy".into());
        }
        hash.update(&buffer[..length]);
    }
    if bytes == 0 {
        return Err("Antigravity executable is empty".into());
    }
    Ok(BinaryFile {
        sha256: hex_digest(&hash.finalize()),
        bytes,
    })
}

async fn verify_payload(path: &Path) -> Result<BTreeMap<String, BinaryFile>, String> {
    let mut identities = BTreeMap::new();
    for (name, identifier) in EXECUTABLES {
        let file = canonical_managed_file(path, &path.join(name), "Antigravity executable")?;
        if !fs::symlink_metadata(path.join(name))
            .map_err(|error| error.to_string())?
            .file_type()
            .is_file()
        {
            return Err("Antigravity executable is not a regular file".into());
        }
        // The same Apple anchor + publisher + identifier policy as other managed runtimes.
        tokio::time::timeout(
            Duration::from_secs(30),
            verify_code_signature(&file, GOOGLE_TEAM_ID, identifier, "Antigravity executable"),
        )
        .await
        .map_err(|_| "Antigravity signature verification timed out")??;
        let identity = file_identity(&file)?;
        identities.insert(name.to_owned(), identity);
    }
    Ok(identities)
}

fn validate_identity_response(value: &serde_json::Value, version: &str) -> Result<(), String> {
    if value["jsonrpc"] != "2.0"
        || value["id"] != 1
        || value.get("error").is_some()
        || value["result"]["protocolVersion"] != 1
        || value["result"]["agentInfo"]["name"] != "antigravity-acp"
        || value["result"]["agentInfo"]["version"] != version
    {
        return Err(
            "signed Antigravity executable did not report the exact Registry ACP identity".into(),
        );
    }
    Ok(())
}

struct IdentityProcess {
    child: tokio::process::Child,
    group: Option<i32>,
}
impl IdentityProcess {
    fn terminate_group(&mut self) {
        if let Some(group) = self.group.take() {
            // The verifier owns this fresh process group, including startup descendants.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    }
}
impl Drop for IdentityProcess {
    fn drop(&mut self) {
        self.terminate_group();
        let _ = self.child.start_kill();
    }
}

/// Unlike the npm adapters this executable has no --version contract.
async fn verify_acp_identity(command: &Path, version: &str) -> Result<(), String> {
    let environment = managed_process_environment(
        command,
        crate::agent_environment::managed_seed().map_err(|error| error.to_string())?,
    )?;
    let mut process = Command::new(command);
    process
        .env_clear()
        .envs(environment)
        .current_dir(
            command
                .parent()
                .ok_or("Antigravity command has no parent")?,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    process.process_group(0);
    let child = process
        .spawn()
        .map_err(|error| format!("unable to verify Antigravity ACP identity: {error}"))?;
    let mut process = IdentityProcess {
        group: child.id().map(|pid| pid as i32),
        child,
    };
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let mut stdin = process.child.stdin.take().ok_or("Antigravity identity probe has no stdin")?;
        stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\"clientInfo\":{\"name\":\"lens-runtime-verifier\",\"version\":\"1\"},\"clientCapabilities\":{}}}\n")
            .await.map_err(|error| error.to_string())?;
        stdin.flush().await.map_err(|error| error.to_string())?;
        let stdout = process.child.stdout.take().ok_or("Antigravity identity probe has no stdout")?;
        let mut reader = BufReader::new(stdout.take(IDENTITY_MAX_BYTES + 1));
        let mut total = 0_u64;
        for _ in 0..16 {
            let mut line = Vec::new();
            let length = reader.read_until(b'\n', &mut line).await.map_err(|error| error.to_string())?;
            total += length as u64;
            if length == 0 || total > IDENTITY_MAX_BYTES { return Err("Antigravity identity response is missing or oversized".into()); }
            let value: serde_json::Value = serde_json::from_slice(&line).map_err(|_| "Antigravity identity response is invalid JSON")?;
            if value.get("id") == Some(&serde_json::json!(1)) {
                return validate_identity_response(&value, version);
            }
            // A prompt-free verifier grants no capabilities and never handles requests.
            if value.get("id").is_some() { return Err("Antigravity requested an effect during identity verification".into()); }
        }
        Err("Antigravity identity response did not arrive within the message limit".into())
    }).await.map_err(|_| "Antigravity ACP identity verification timed out".to_owned());
    process.terminate_group();
    let _ = process.child.kill().await;
    result?
}

fn validate_record(record: &BinaryRecord, now: i64) -> Result<(), String> {
    if record.schema_version != RECORD_VERSION
        || record.distribution != "google_signed_archive"
        || record.registry_id != "antigravity-acp"
        || record.target != TARGET
        || record.archive_url != archive_url(&record.adapter_version)?
        || record
            .registry_sha256
            .as_deref()
            .is_some_and(|hash| !valid_sha256(hash) || hash != record.archive_sha256)
        || !valid_sha256(&record.archive_sha256)
        || record.archive_bytes == 0
        || record.archive_bytes > ARCHIVE_MAX_BYTES
        || record.observed_at_unix_seconds > now
        || !archive_is_mature(
            &record.archive_last_modified,
            record.observed_at_unix_seconds,
        )?
        || record.files.len() != EXECUTABLES.len()
        || record.files.iter().any(|(name, file)| {
            !allowed_name(name)
                || !valid_sha256(&file.sha256)
                || file.bytes == 0
                || file.bytes > EXECUTABLE_MAX_BYTES
        })
        || record.files.values().map(|file| file.bytes).sum::<u64>() > PAYLOAD_MAX_BYTES
    {
        return Err(
            "managed Antigravity install record does not match binary distribution policy".into(),
        );
    }
    Ok(())
}

pub(super) async fn verify_installed(path: &Path) -> Result<ResolvedAgentRuntime, String> {
    let record_path =
        canonical_managed_file(path, &path.join(RECORD), "Antigravity install record")?;
    if fs::metadata(&record_path)
        .map_err(|error| error.to_string())?
        .len()
        > 16 * 1024
    {
        return Err("Antigravity install record exceeds size policy".into());
    }
    let record: BinaryRecord =
        serde_json::from_slice(&fs::read(record_path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("invalid Antigravity install record: {error}"))?;
    validate_record(&record, now_seconds()?)?;
    let entries = fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .take(EXECUTABLES.len() + 2)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if entries.len() != EXECUTABLES.len() + 1
        || entries.iter().any(|entry| {
            let name = entry.file_name();
            name.to_str()
                .is_none_or(|name| name != RECORD && !allowed_name(name))
                || !entry.file_type().is_ok_and(|kind| kind.is_file())
        })
    {
        return Err("Antigravity installation contains unverified files".into());
    }
    if verify_payload(path).await? != record.files {
        return Err("managed Antigravity executable content was modified".into());
    }
    let command =
        canonical_managed_file(path, &path.join(EXECUTABLES[0].0), "Antigravity adapter")?;
    verify_acp_identity(&command, &record.adapter_version).await?;
    Ok(ResolvedAgentRuntime {
        kind: AgentKind::Antigravity,
        adapter_name: "antigravity-acp",
        adapter_version: record.adapter_version,
        command,
        args: vec![],
        installation: None,
    })
}

pub(super) async fn install_candidate<R: tauri::Runtime>(
    app: &AppHandle<R>,
    root: &Path,
    release: &Release,
    operation: Uuid,
) -> Result<CandidateInstallation, String> {
    ensure_supported_target()?;
    let version = &release.version;
    update_agent_runtime(app, operation, |state| {
        state.stage = AgentRuntimeStage::Downloading;
        state.version = Some(version.to_owned());
        state.message = Some(format!(
            "Downloading official Google Antigravity {version}…"
        ));
        state.downloaded_bytes = 0;
        state.total_bytes = None;
    })?;
    let staging = StagingCleanup::create(root, "Antigravity")?;
    let archive = staging.path.join("adapter.zip");
    let Some(download) = download(app, operation, release, &archive).await? else {
        return Ok(CandidateInstallation::BlockedByArchiveAgePolicy);
    };
    update_agent_runtime(app, operation, |state| {
        state.stage = AgentRuntimeStage::Verifying;
        state.message = Some("Verifying Google's signed Antigravity executables…".into());
    })?;
    let payload = staging.path.join("agent");
    let worker_staging = Arc::clone(&staging);
    let worker_payload = payload.clone();
    tokio::task::spawn_blocking(move || {
        // A cancelled future must not remove a directory the extractor still owns.
        let _staging = worker_staging;
        extract(&archive, &worker_payload)
    })
    .await
    .map_err(|error| format!("Antigravity extraction task failed: {error}"))??;
    let files = verify_payload(&payload).await?;
    verify_acp_identity(&payload.join(EXECUTABLES[0].0), version).await?;
    let record = BinaryRecord {
        schema_version: RECORD_VERSION,
        distribution: "google_signed_archive".into(),
        registry_id: "antigravity-acp".into(),
        adapter_version: version.to_owned(),
        target: TARGET.into(),
        archive_url: release.archive_url.clone(),
        archive_sha256: download.sha256,
        registry_sha256: release.sha256.clone(),
        archive_bytes: download.bytes,
        archive_last_modified: download.last_modified,
        observed_at_unix_seconds: download.observed,
        files,
    };
    validate_record(&record, now_seconds()?)?;
    fs::write(
        payload.join(RECORD),
        serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let id = Uuid::new_v4().to_string();
    let destination = install_root(root, AgentKind::Antigravity, &id)?;
    fs::create_dir_all(
        destination
            .parent()
            .ok_or("Antigravity installation has no parent")?,
    )
    .map_err(|error| error.to_string())?;
    {
        let _guard = selector_mutex()
            .lock()
            .map_err(|_| "runtime selector unavailable")?;
        fs::rename(&payload, &destination).map_err(|error| error.to_string())?;
        let mut selector = read_selector(root, AgentKind::Antigravity)?;
        selector.candidate = Some(id.clone());
        write_selector(root, AgentKind::Antigravity, &selector)?;
    }
    load_runtime(root, AgentKind::Antigravity, &id)
        .await
        .map(CandidateInstallation::Installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

    fn registry(archive: &str, extra: serde_json::Value) -> Vec<u8> {
        let mut binary = serde_json::json!({"archive": archive, "cmd": "./agy_acp_server.par"});
        for (key, value) in extra.as_object().unwrap() {
            binary[key] = value.clone();
        }
        serde_json::to_vec(&serde_json::json!({"version": "1.0.0", "agents": [{
            "id": "antigravity-acp", "version": "1.2.1",
            "distribution": {"binary": {"darwin-aarch64": binary}}
        }]}))
        .unwrap()
    }

    #[test]
    fn registry_binary_identity_is_explicit_and_digest_is_retained() {
        let url = archive_url("1.2.1").unwrap();
        let RegistryRelease::Antigravity(release) = validate_registry_release(
            &registry(&url, serde_json::json!({"sha256": "AB".repeat(32)})),
            AgentKind::Antigravity,
        )
        .unwrap() else {
            panic!("wrong distribution");
        };
        assert_eq!(release.sha256, Some("ab".repeat(32)));
        for (archive, extra) in [
            (url.clone(), serde_json::json!({"sha256": "bad"})),
            (url.clone(), serde_json::json!({"cmd": "./other"})),
            (url.clone(), serde_json::json!({"args": ["--unsafe"]})),
            (url.clone(), serde_json::json!({"env": {"ENV": "value"}})),
            (
                url.replace("dl.google.com", "attacker.invalid"),
                serde_json::json!({}),
            ),
            (url.replace("1.2.1", "1.2.0"), serde_json::json!({})),
            (format!("{url}?redirect=elsewhere"), serde_json::json!({})),
        ] {
            assert!(
                validate_registry_release(&registry(&archive, extra), AgentKind::Antigravity)
                    .is_err()
            );
        }
        assert!(manifest(AgentKind::Antigravity, "1.2.1").is_err());
    }

    #[test]
    fn unrelated_binary_shapes_do_not_change_npm_registry_resolution() {
        let entries = serde_json::json!({"version": "1.0.0", "agents": [
            {"id": "claude-acp", "version": "1.2.3", "distribution": {"npx": {"package": "@agentclientprotocol/claude-agent-acp@1.2.3"}}},
            {"id": "other", "version": "1.2.3", "distribution": {"binary": "future-schema"}}
        ]});
        assert_eq!(
            validate_registry_entry(&serde_json::to_vec(&entries).unwrap(), AgentKind::Claude)
                .unwrap(),
            "1.2.3"
        );
        assert!(validate_registry_entry(
            &registry(
                &archive_url("1.2.1").unwrap(),
                serde_json::json!({"unexpected": true})
            ),
            AgentKind::Antigravity
        )
        .is_err());
    }

    #[test]
    fn archive_age_rejects_missing_invalid_future_and_young_dates() {
        let modified = "Wed, 23 Sep 2026 18:07:27 GMT";
        let time = DateTime::parse_from_rfc2822(modified).unwrap().timestamp();
        assert!(!archive_is_mature(modified, time + 3599).unwrap());
        assert!(archive_is_mature(modified, time + 3600).unwrap());
        assert!(archive_is_mature(modified, time + 3601).unwrap());
        assert!(archive_is_mature(modified, time - 1).is_err());
        assert!(archive_is_mature("", time).is_err());
        assert!(archive_is_mature("not a date", time).is_err());
    }

    fn zip_fixture(names: &[&str], symlink: bool) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o755);
        for (index, name) in names.iter().enumerate() {
            if symlink && index == 0 {
                zip.add_symlink(*name, "/outside", options).unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
                zip.write_all(b"synthetic executable").unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn extraction_writes_only_two_regular_basenames_and_rejects_bad_crc() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("fixture.zip");
        let expected = [EXECUTABLES[0].0, EXECUTABLES[1].0];
        let good = zip_fixture(&expected, false);
        fs::write(&archive, &good).unwrap();
        let destination = root.path().join("accepted");
        extract(&archive, &destination).unwrap();
        assert_eq!(
            fs::read(destination.join(expected[0])).unwrap(),
            b"synthetic executable"
        );
        for (index, names) in [
            vec![expected[0]],
            vec!["../escape", expected[1]],
            vec!["/absolute", expected[1]],
            vec![expected[0], expected[1], "unexpected"],
            vec!["directory/", expected[1]],
        ]
        .into_iter()
        .enumerate()
        {
            fs::write(&archive, zip_fixture(&names, false)).unwrap();
            assert!(extract(&archive, &root.path().join(format!("rejected-{index}"))).is_err());
        }
        fs::write(&archive, zip_fixture(&expected, true)).unwrap();
        assert!(extract(&archive, &root.path().join("symlink")).is_err());
        let mut corrupt = good;
        let offset = corrupt
            .windows(b"synthetic executable".len())
            .position(|window| window == b"synthetic executable")
            .unwrap();
        corrupt[offset] ^= 1;
        fs::write(&archive, corrupt).unwrap();
        assert!(extract(&archive, &root.path().join("bad-crc")).is_err());
        assert!(!root.path().join("escape").exists());
    }

    #[test]
    fn extraction_rejects_duplicate_names_and_forged_expanded_sizes_before_writing() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("fixture.zip");
        let alias = "agy_acp_serveq.par";
        assert_eq!(alias.len(), EXECUTABLES[0].0.len());
        let mut duplicate = zip_fixture(&[EXECUTABLES[0].0, alias], false);
        let offsets = duplicate
            .windows(alias.len())
            .enumerate()
            .filter_map(|(offset, value)| (value == alias.as_bytes()).then_some(offset))
            .collect::<Vec<_>>();
        for offset in offsets {
            duplicate[offset..offset + alias.len()].copy_from_slice(EXECUTABLES[0].0.as_bytes());
        }
        fs::write(&archive, duplicate).unwrap();
        assert!(extract(&archive, &root.path().join("duplicate")).is_err());
        let mut oversized = zip_fixture(&[EXECUTABLES[0].0, EXECUTABLES[1].0], false);
        let central = oversized
            .windows(4)
            .position(|value| value == b"PK\x01\x02")
            .unwrap();
        oversized[central + 24..central + 28]
            .copy_from_slice(&((EXECUTABLE_MAX_BYTES + 1) as u32).to_le_bytes());
        fs::write(&archive, oversized).unwrap();
        let destination = root.path().join("oversized");
        assert!(extract(&archive, &destination).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn malformed_final_directory_cannot_fall_back_to_an_earlier_archive() {
        let root = tempfile::tempdir().unwrap();
        let mut bytes = zip_fixture(&[EXECUTABLES[0].0, EXECUTABLES[1].0], false);
        let original_length = bytes.len();
        let mut footer: [u8; 22] = bytes[original_length - 22..].try_into().unwrap();
        bytes.extend_from_slice(&[0_u8; 92]);
        footer[12..16].copy_from_slice(&92_u32.to_le_bytes());
        footer[16..20].copy_from_slice(&(original_length as u32).to_le_bytes());
        bytes.extend_from_slice(&footer);
        // Demonstrate the upstream fallback this boundary must suppress.
        assert_eq!(
            ZipArchive::new(Cursor::new(bytes.clone())).unwrap().len(),
            2
        );
        let path = root.path().join("fallback.zip");
        fs::write(&path, bytes).unwrap();
        let destination = root.path().join("must-not-extract");
        assert!(extract(&path, &destination).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn identity_requires_exact_agent_version_and_protocol() {
        let value = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {
            "protocolVersion": 1, "agentInfo": {"name": "antigravity-acp", "version": "1.2.1"}
        }});
        validate_identity_response(&value, "1.2.1").unwrap();
        assert!(validate_identity_response(&value, "1.2.2").is_err());
        let mut incompatible = value.clone();
        incompatible["result"]["protocolVersion"] = serde_json::json!(2);
        assert!(validate_identity_response(&incompatible, "1.2.1").is_err());
        incompatible = value;
        incompatible["result"]["agentInfo"]["name"] = serde_json::json!("other");
        assert!(validate_identity_response(&incompatible, "1.2.1").is_err());
    }

    #[test]
    fn binary_record_rejects_provenance_and_file_policy_mutation() {
        let modified = "Wed, 23 Sep 2026 18:07:27 GMT";
        let observed =
            DateTime::parse_from_rfc2822(modified).unwrap().timestamp() + MIN_ARCHIVE_AGE_SECONDS;
        let mut record = BinaryRecord {
            schema_version: RECORD_VERSION,
            distribution: "google_signed_archive".into(),
            registry_id: "antigravity-acp".into(),
            adapter_version: "1.2.1".into(),
            target: TARGET.into(),
            archive_url: archive_url("1.2.1").unwrap(),
            archive_sha256: "ab".repeat(32),
            registry_sha256: None,
            archive_bytes: 100,
            archive_last_modified: modified.into(),
            observed_at_unix_seconds: observed,
            files: EXECUTABLES
                .iter()
                .map(|(name, _)| {
                    (
                        name.to_string(),
                        BinaryFile {
                            sha256: "12".repeat(32),
                            bytes: 10,
                        },
                    )
                })
                .collect(),
        };
        validate_record(&record, observed).unwrap();
        record.registry_sha256 = Some("cd".repeat(32));
        assert!(validate_record(&record, observed).is_err());
        record.registry_sha256 = None;
        record.files.insert(
            "extra".into(),
            BinaryFile {
                sha256: "12".repeat(32),
                bytes: 10,
            },
        );
        assert!(validate_record(&record, observed).is_err());
        record.files.remove("extra");
        record.observed_at_unix_seconds += 1;
        assert!(validate_record(&record, observed).is_err());
    }

    #[tokio::test]
    async fn identity_probe_drop_terminates_its_startup_descendants() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "sleep 60 & echo $!; wait"])
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let child = command.spawn().unwrap();
        let mut guard = IdentityProcess {
            group: child.id().map(|pid| pid as i32),
            child,
        };
        let stdout = guard.child.stdout.take().unwrap();
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(3),
            BufReader::new(stdout).read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        let descendant: i32 = line.trim().parse().unwrap();
        assert_eq!(
            unsafe { libc::kill(descendant, 0) },
            0,
            "fixture descendant must start alive"
        );
        drop(guard);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if unsafe { libc::kill(descendant, 0) } == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    break;
                }
                // A container's PID 1 may not reap an orphan immediately. A zombie
                // has terminated and cannot execute, even though kill(pid, 0) succeeds.
                let status = Command::new("/bin/ps")
                    .args(["-o", "stat=", "-p", &descendant.to_string()])
                    .stdin(Stdio::null())
                    .kill_on_drop(true)
                    .output()
                    .await
                    .unwrap();
                if String::from_utf8_lossy(&status.stdout)
                    .trim()
                    .starts_with('Z')
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("startup descendant survived cancelled verifier");
    }

    #[tokio::test]
    #[ignore = "downloads and verifies official signed Antigravity in disposable LENS_ANTIGRAVITY_RUNTIME_ROOT; existing Google authentication required for startup admission"]
    async fn install_official_archive_without_node_and_confirm_after_real_acp_startup() {
        let root = fs::canonicalize(PathBuf::from(
            std::env::var_os("LENS_ANTIGRAVITY_RUNTIME_ROOT").expect("disposable root"),
        ))
        .unwrap();
        let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
        assert!(
            (root.starts_with(temp) || root.starts_with("/private/tmp"))
                && root
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("lens-")
        );
        assert!(!root.join("node").exists() && !root.join("pnpm").exists());
        let workspace = root.join("synthetic-workspace");
        fs::create_dir_all(&workspace).unwrap();
        let mut state = crate::test_support::state();
        state.store = crate::store::ConfigStore::at_path(root.join("test-settings.json"));
        {
            let mut snapshot = state.runtime.write().unwrap();
            snapshot.config.agent = AgentKind::Antigravity;
            snapshot.config.working_directory = workspace;
        }
        struct AdmissionTray;
        impl crate::ui::TrayOutput<tauri::test::MockRuntime> for AdmissionTray {
            fn apply(
                &self,
                _: &AppHandle<tauri::test::MockRuntime>,
                _: crate::ui::TrayMenuPresentation,
            ) -> Result<(), String> {
                Ok(())
            }
        }
        let app = tauri::test::mock_builder()
            .manage(state)
            .manage(crate::agent::AgentServices::<tauri::test::MockRuntime>(
                Arc::new(crate::agent::DefaultAgentHost),
            ))
            .manage(crate::ui::TrayPresentation::<tauri::test::MockRuntime>(
                Arc::new(AdmissionTray),
            ))
            .build(crate::product_context())
            .unwrap();
        let (runtime, operation, _) =
            resolve_at(app.handle(), AgentKind::Antigravity, true, true, &root)
                .await
                .unwrap();
        assert!(runtime.args.is_empty());
        assert_eq!(runtime.adapter_name, "antigravity-acp");
        assert!(!root.join("node").exists() && !root.join("pnpm").exists());
        let installation = runtime.installation.as_ref().unwrap();
        let path = install_root(&root, AgentKind::Antigravity, &installation.id).unwrap();
        let original_record = fs::read(path.join(RECORD)).unwrap();
        let mut tampered: serde_json::Value = serde_json::from_slice(&original_record).unwrap();
        tampered["files"][EXECUTABLES[1].0]["sha256"] = serde_json::json!("00".repeat(32));
        fs::write(path.join(RECORD), serde_json::to_vec(&tampered).unwrap()).unwrap();
        let rejected = verify_installed(&path).await.is_err();
        fs::write(path.join(RECORD), original_record).unwrap();
        assert!(rejected, "modified helper identity must not be accepted");
        let tamper_root = tempfile::tempdir().unwrap();
        let tampered_helper = tamper_root.path().join("helper");
        fs::copy(path.join(EXECUTABLES[1].0), &tampered_helper).unwrap();
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&tampered_helper)
            .unwrap();
        file.seek(SeekFrom::Start(4096)).unwrap();
        let mut byte = [0_u8];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 1;
        file.seek(SeekFrom::Start(4096)).unwrap();
        file.write_all(&byte).unwrap();
        file.flush().unwrap();
        assert!(
            verify_code_signature(
                &tampered_helper,
                GOOGLE_TEAM_ID,
                EXECUTABLES[1].1,
                "tampered fixture helper"
            )
            .await
            .is_err(),
            "changed helper bytes must fail Google's code signature"
        );

        let verified = crate::agent::verify_managed_runtime(app.handle(), runtime.clone())
            .await
            .unwrap();
        commit_verified_update(app.handle(), &runtime, verified).unwrap();
        let selected = resolve_antigravity_fixture(&root).await.unwrap();
        assert_eq!(selected.installation.as_ref().unwrap().id, installation.id);
        assert_eq!(
            read_selector(&root, AgentKind::Antigravity)
                .unwrap()
                .current
                .as_deref(),
            Some(installation.id.as_str())
        );
        assert!(read_selector(&root, AgentKind::Antigravity)
            .unwrap()
            .candidate
            .is_none());
        let (unchanged, _, status) =
            resolve_at(app.handle(), AgentKind::Antigravity, true, true, &root)
                .await
                .unwrap();
        assert_eq!(status, ResolutionStatus::Existing);
        assert_eq!(unchanged.installation.as_ref().unwrap().id, installation.id);
        assert!(!root.join("node").exists() && !root.join("pnpm").exists());

        eprintln!(
            "Antigravity managed install confirmed: root={} version={} operation={operation}",
            root.display(),
            selected.adapter_version
        );
    }
}
