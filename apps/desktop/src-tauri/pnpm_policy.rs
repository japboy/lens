//! Development configuration owns pnpm; its lock owns the native archive identity.
use std::{fs, path::Path};
pub const INPUTS: [&str; 4] = [
    "mise.toml",
    "mise.lock",
    "package.json",
    "apps/desktop/package.json",
];
pub const HISTORY_INPUT: &str = "apps/desktop/src-tauri/agent-runtime/pnpm-history.toml";
#[derive(Debug, PartialEq, Eq)]
pub struct PnpmPolicy {
    pub version: String,
    pub archive_name: String,
    pub archive_url: String,
    pub archive_sha256: String,
    pub previous: Vec<(String, String)>,
}
pub fn read(root: &Path) -> Result<PnpmPolicy, String> {
    let texts = INPUTS
        .iter()
        .map(|path| fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut policy = parse(&texts[0], &texts[1])?;
    for (path, text) in INPUTS[2..].iter().zip(&texts[2..]) {
        validate_package(path, text, &policy.version)?;
    }
    policy.previous = parse_history(
        &fs::read_to_string(root.join(HISTORY_INPUT))
            .map_err(|e| format!("{HISTORY_INPUT}: {e}"))?,
    )?;
    let current_digest = format!("sha256:{}", policy.archive_sha256);
    if policy
        .previous
        .iter()
        .any(|(version, digest)| version == &policy.version && digest != &current_digest)
    {
        return Err("conflicting current and historical pnpm digest".into());
    }
    Ok(policy)
}
pub fn validate_package(path: &str, text: &str, version: &str) -> Result<(), String> {
    let package: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("{path}: {e}"))?;
    let manager = package.get("packageManager");
    if package["engines"]["pnpm"].as_str() != Some(version)
        || ((path == "package.json" || manager.is_some())
            && manager.and_then(serde_json::Value::as_str)
                != Some(format!("pnpm@{version}").as_str()))
    {
        return Err(format!(
            "{path}: engines.pnpm and packageManager must equal mise pnpm version {version}"
        ));
    }
    Ok(())
}
fn exact_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}
fn hex_digest(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn parse(config: &str, lock: &str) -> Result<PnpmPolicy, String> {
    let config: toml::Value = toml::from_str(config).map_err(|e| format!("mise.toml: {e}"))?;
    let version = config
        .get("tools")
        .and_then(|v| v.get("pnpm"))
        .and_then(toml::Value::as_str)
        .ok_or("mise.toml: tools.pnpm must be one exact version")?;
    if !exact_version(version) || version.split('.').next() != Some("12") {
        return Err("managed pnpm requires an exact stable 12.x.y native version".into());
    }
    let lock: toml::Value = toml::from_str(lock).map_err(|e| format!("mise.lock: {e}"))?;
    let entries = lock
        .get("tools")
        .and_then(|v| v.get("pnpm"))
        .and_then(toml::Value::as_array)
        .ok_or("mise.lock: pnpm requires exactly one entry")?;
    let [entry] = entries.as_slice() else {
        return Err("mise.lock: pnpm requires exactly one entry".into());
    };
    if entry.get("version").and_then(toml::Value::as_str) != Some(version)
        || entry.get("backend").and_then(toml::Value::as_str) != Some("aqua:pnpm/pnpm")
    {
        return Err(
            "mise.lock: pnpm version/backend must match mise.toml and aqua:pnpm/pnpm".into(),
        );
    }
    let archive_name = "pnpm-darwin-arm64.tar.gz".to_string();
    let archive_url =
        format!("https://github.com/pnpm/pnpm/releases/download/v{version}/{archive_name}");
    let artifact = entry
        .get("platforms.macos-arm64")
        .ok_or("mise.lock: missing managed pnpm macos-arm64 artifact")?;
    if artifact.get("url").and_then(toml::Value::as_str) != Some(archive_url.as_str()) {
        return Err(
            "mise.lock: pnpm URL must be the official versioned darwin-arm64 tar.gz".into(),
        );
    }
    let checksum = artifact
        .get("checksum")
        .and_then(toml::Value::as_str)
        .and_then(|v| v.strip_prefix("sha256:"))
        .ok_or("mise.lock: pnpm checksum must use sha256")?;
    if !hex_digest(checksum, 64) {
        return Err("mise.lock: pnpm checksum must be 64 lowercase hexadecimal digits".into());
    }
    Ok(PnpmPolicy {
        version: version.into(),
        archive_name,
        archive_url,
        archive_sha256: checksum.into(),
        previous: Vec::new(),
    })
}
pub fn parse_history(text: &str) -> Result<Vec<(String, String)>, String> {
    let history: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
    let table = history.as_table().ok_or("pnpm history must be a table")?;
    if table.keys().any(|key| key != "previous") {
        return Err("pnpm history owns only previous identities".into());
    }
    let entries = table
        .get("previous")
        .and_then(toml::Value::as_array)
        .ok_or("pnpm history requires previous array")?;
    let mut identities = Vec::new();
    for entry in entries {
        let entry = entry
            .as_table()
            .ok_or("pnpm history entry must be a table")?;
        if entry.len() != 2
            || entry
                .keys()
                .any(|key| !["version", "archive_digest"].contains(&key.as_str()))
        {
            return Err("pnpm history entry requires version and archive_digest only".into());
        }
        let version = entry
            .get("version")
            .and_then(toml::Value::as_str)
            .ok_or("missing historical pnpm version")?;
        let digest = entry
            .get("archive_digest")
            .and_then(toml::Value::as_str)
            .ok_or("missing historical pnpm digest")?;
        let valid_digest = match version.split('.').next() {
            Some("11") => digest
                .strip_prefix("sha512:")
                .is_some_and(|v| hex_digest(v, 128)),
            Some("12") => digest
                .strip_prefix("sha256:")
                .is_some_and(|v| hex_digest(v, 64)),
            _ => false,
        };
        if !exact_version(version) || !valid_digest {
            return Err("unsupported or noncanonical historical pnpm identity".into());
        }
        if identities.iter().any(|(approved, _)| approved == version) {
            return Err("duplicate historical pnpm version".into());
        }
        identities.push((version.to_string(), digest.to_string()));
    }
    Ok(identities)
}
