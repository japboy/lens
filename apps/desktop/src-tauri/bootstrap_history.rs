//! Load-only identities remain trusted repository declarations, never record self-assertions.
use std::{fs, path::Path};
pub const INPUT: &str = "apps/desktop/src-tauri/agent-runtime/node-history.toml";
pub fn read(root: &Path) -> Result<Vec<(String, String)>, String> {
    parse(&fs::read_to_string(root.join(INPUT)).map_err(|e| e.to_string())?)
}
pub fn parse(text: &str) -> Result<Vec<(String, String)>, String> {
    let value: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
    let table = value.as_table().ok_or("Node history must be a table")?;
    if table.len() != 1 {
        return Err("Node history permits only previous identities".into());
    }
    let entries = table
        .get("previous")
        .and_then(toml::Value::as_array)
        .ok_or("Node previous identities must be an array")?;
    let mut identities = Vec::new();
    for entry in entries {
        let entry = entry.as_table().ok_or("Node identity must be a table")?;
        if entry.len() != 2 {
            return Err("Node identity requires version and archive_sha256".into());
        }
        let version = entry
            .get("version")
            .and_then(toml::Value::as_str)
            .ok_or("missing previous Node version")?;
        let parts: Vec<_> = version.split('.').collect();
        if parts.len() != 3
            || parts.iter().any(|p| {
                p.is_empty()
                    || !p.bytes().all(|b| b.is_ascii_digit())
                    || (p.len() > 1 && p.starts_with('0'))
            })
        {
            return Err("previous Node version must be exact stable semver".into());
        }
        let digest = entry
            .get("archive_sha256")
            .and_then(toml::Value::as_str)
            .ok_or("missing previous Node digest")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("previous Node digest must be canonical SHA256 hex".into());
        }
        if identities.iter().any(|(v, _)| v == version) {
            return Err("duplicate previous Node version".into());
        }
        identities.push((version.to_owned(), digest.to_owned()));
    }
    Ok(identities)
}
