//! The output tool is a bundled child process, never a user-global MCP setting.
use agent_client_protocol::schema::v1::{McpServer, McpServerStdio, NewSessionRequest};
use std::path::{Path, PathBuf};

const SERVER_NAME: &str = "lens_output";
const EXECUTABLE_NAME: &str = "lens-output-mcp";

pub(crate) fn bundled_executable() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Cannot locate Lens executable: {error}"))?;
    let sibling = sibling_executable(&executable)?;
    let metadata = std::fs::symlink_metadata(&sibling)
        .map_err(|error| format!("Bundled HTML output server is unavailable: {error}"))?;
    if !metadata.is_file() {
        return Err("Bundled HTML output server must be a regular executable file".into());
    }
    Ok(sibling)
}

fn sibling_executable(executable: &Path) -> Result<PathBuf, String> {
    if !executable.is_absolute() {
        return Err("Lens executable path must be absolute".into());
    }
    executable
        .parent()
        .map(|parent| parent.join(EXECUTABLE_NAME))
        .ok_or_else(|| "Lens executable has no parent directory".into())
}

pub(crate) fn session_request(cwd: &Path, executable: PathBuf) -> NewSessionRequest {
    NewSessionRequest::new(cwd).mcp_servers(vec![McpServer::Stdio(McpServerStdio::new(
        SERVER_NAME,
        executable,
    ))])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_server_is_a_sibling_not_a_path_lookup() {
        for (lens, expected) in [
            (
                "/Applications/Lens.app/Contents/MacOS/lens",
                "/Applications/Lens.app/Contents/MacOS/lens-output-mcp",
            ),
            (
                "/build/target/debug/lens",
                "/build/target/debug/lens-output-mcp",
            ),
        ] {
            assert_eq!(
                sibling_executable(Path::new(lens)).unwrap(),
                PathBuf::from(expected)
            );
        }
        assert!(sibling_executable(Path::new("lens")).is_err());
    }

    #[test]
    fn registration_is_explicit_session_local_stdio_without_extra_authority() {
        let request = session_request(
            Path::new("/workspace"),
            PathBuf::from("/bundle/lens-output-mcp"),
        );
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["cwd"], "/workspace");
        assert_eq!(
            json["mcpServers"],
            serde_json::json!([{
                "name": "lens_output",
                "command": "/bundle/lens-output-mcp",
                "args": [],
                "env": []
            }])
        );
        assert!(json.get("additionalDirectories").is_none());
    }
}
