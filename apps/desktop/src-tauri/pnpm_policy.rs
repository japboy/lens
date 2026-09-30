//! One declaration owns the managed npm tarball identity, independent of development pnpm.
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{fs, path::Path};
pub const INPUT: &str = "apps/desktop/src-tauri/agent-runtime/pnpm.toml";
#[derive(Debug, PartialEq, Eq)]
pub struct PnpmPolicy {
    pub version: String,
    pub archive_name: String,
    pub archive_url: String,
    pub archive_sha512: String,
    pub previous: Vec<(String, String)>,
}
pub fn read(root: &Path) -> Result<PnpmPolicy, String> {
    parse(&fs::read_to_string(root.join(INPUT)).map_err(|e| e.to_string())?)
}
pub fn parse(text: &str) -> Result<PnpmPolicy, String> {
    let declaration: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
    let table = declaration
        .as_table()
        .ok_or("pnpm policy must be a table")?;
    if table
        .keys()
        .any(|key| !["version", "integrity", "previous"].contains(&key.as_str()))
    {
        return Err("pnpm policy requires only version and integrity".into());
    }
    let mut previous = Vec::new();
    if let Some(entries) = table.get("previous") {
        let entries = entries
            .as_array()
            .ok_or("previous pnpm identities must be an array")?;
        for entry in entries {
            let identity = parse(&toml::to_string(entry).map_err(|e| e.to_string())?)?;
            if !identity.previous.is_empty() {
                return Err("nested pnpm history is not allowed".into());
            }
            if previous
                .iter()
                .any(|(version, _)| version == &identity.version)
            {
                return Err("duplicate previous pnpm version".into());
            }
            previous.push((identity.version, identity.archive_sha512));
        }
    }
    let version = table
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or("missing pnpm version")?;
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() != 3
        || parts[0] != "11"
        || parts.iter().any(|p| {
            p.is_empty()
                || !p.bytes().all(|b| b.is_ascii_digit())
                || (p.len() > 1 && p.starts_with('0'))
        })
    {
        return Err("managed pnpm requires an exact stable 11.x.y version".into());
    }
    let integrity = table
        .get("integrity")
        .and_then(toml::Value::as_str)
        .ok_or("missing pnpm integrity")?;
    let encoded = integrity
        .strip_prefix("sha512-")
        .ok_or("pnpm integrity must use sha512 SRI")?;
    let digest = STANDARD.decode(encoded).map_err(|e| e.to_string())?;
    if digest.len() != 64 || STANDARD.encode(&digest) != encoded {
        return Err("invalid canonical SHA-512 SRI".into());
    }
    Ok(PnpmPolicy {
        version: version.into(),
        archive_name: format!("pnpm-{version}.tgz"),
        archive_url: format!("https://npm.flatt.tech/pnpm/-/pnpm-{version}.tgz"),
        previous,
        archive_sha512: digest.iter().map(|b| format!("{b:02x}")).collect(),
    })
}
