//! Build-time policy for the sole supported managed Node artifact: macOS arm64.
//! Development configuration owns the version; its lock owns artifact provenance.
use std::{fs, path::Path};

pub const INPUTS: [&str; 4] = [
    "mise.toml",
    "mise.lock",
    "package.json",
    "apps/desktop/package.json",
];

#[derive(Debug, PartialEq, Eq)]
pub struct NodePolicy {
    pub version: String,
    pub target: &'static str,
    pub archive_name: String,
    pub archive_root: String,
    pub archive_url: String,
    pub archive_sha256: String,
}

pub fn read(root: &Path) -> Result<NodePolicy, String> {
    let texts: Vec<String> = INPUTS
        .iter()
        .map(|path| fs::read_to_string(root.join(path)).map_err(|error| format!("{path}: {error}")))
        .collect::<Result<_, _>>()?;
    let policy = parse(&texts[0], &texts[1])?;
    for (path, text) in INPUTS[2..].iter().zip(&texts[2..]) {
        validate_engine(path, text, &policy.version)?;
    }
    Ok(policy)
}

pub fn validate_engine(path: &str, text: &str, version: &str) -> Result<(), String> {
    let package: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("{path}: invalid JSON: {error}"))?;
    if package["engines"]["node"].as_str() != Some(version) {
        return Err(format!(
            "{path}: engines.node must equal mise Node version {version}"
        ));
    }
    Ok(())
}

pub fn parse(config: &str, lock: &str) -> Result<NodePolicy, String> {
    let config: toml::Value =
        toml::from_str(config).map_err(|error| format!("mise.toml: invalid TOML: {error}"))?;
    let version = config
        .get("tools")
        .and_then(|v| v.get("node"))
        .and_then(toml::Value::as_str)
        .ok_or("mise.toml: tools.node must be one exact version string")?;
    let components: Vec<_> = version.split('.').collect();
    if components.len() != 3
        || components.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
        })
    {
        return Err("mise.toml: tools.node must be an exact major.minor.patch version".into());
    }
    let lock: toml::Value =
        toml::from_str(lock).map_err(|error| format!("mise.lock: invalid TOML: {error}"))?;
    let entries = lock
        .get("tools")
        .and_then(|v| v.get("node"))
        .and_then(toml::Value::as_array)
        .ok_or("mise.lock: tools.node must contain exactly one lock entry")?;
    let [entry] = entries.as_slice() else {
        return Err("mise.lock: tools.node must contain exactly one lock entry".into());
    };
    if entry.get("version").and_then(toml::Value::as_str) != Some(version) {
        return Err("mise.lock: Node version must equal mise.toml tools.node".into());
    }
    if entry.get("backend").and_then(toml::Value::as_str) != Some("core:node") {
        return Err("mise.lock: Node backend must be core:node".into());
    }
    // This is the application support policy, deliberately independent of the build host/target.
    let target = "darwin-arm64";
    let archive_root = format!("node-v{version}-{target}");
    let archive_name = format!("{archive_root}.tar.gz");
    let archive_url = format!("https://nodejs.org/dist/v{version}/{archive_name}");
    let artifact = entry
        .get("platforms.macos-arm64")
        .ok_or("mise.lock: missing managed Node platforms.macos-arm64 artifact")?;
    if artifact.get("url").and_then(toml::Value::as_str) != Some(archive_url.as_str()) {
        return Err(
            "mise.lock: managed Node URL must be the official versioned darwin-arm64 tar.gz".into(),
        );
    }
    let checksum = artifact
        .get("checksum")
        .and_then(toml::Value::as_str)
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or("mise.lock: managed Node checksum must use sha256")?;
    if checksum.len() != 64
        || !checksum
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(
            "mise.lock: managed Node checksum must be 64 lowercase hexadecimal digits".into(),
        );
    }
    Ok(NodePolicy {
        version: version.into(),
        target,
        archive_name,
        archive_root,
        archive_url,
        archive_sha256: checksum.into(),
    })
}
