//! Session-local HTTP MCP registration and explicit per-turn publication authority.
use adapter_output_mcp::HttpPublisher;
use agent_client_protocol::schema::v1::{
    ContentBlock, EmbeddedResource, EmbeddedResourceResource, HttpHeader, McpCapabilities,
    McpServer, McpServerHttp, NewSessionRequest, TextContent, TextResourceContents,
};
use agent_client_protocol::Error;
use std::path::Path;
use uuid::Uuid;

/// Goose reports extension failures without failing session/new. Only its
/// explicit failure result is authoritative; absent metadata is ordinary ACP.
pub(crate) fn require_no_publisher_failure(
    agent_name: Option<&str>,
    meta: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<(), Error> {
    let failed = agent_name == Some("goose")
        && meta
            .and_then(|meta| meta.get("extensionResults"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|results| {
                results.iter().any(|result| {
                    result.get("name").and_then(serde_json::Value::as_str) == Some("lens_output")
                        && result.get("success").and_then(serde_json::Value::as_bool) == Some(false)
                })
            });
    if failed {
        // Extension error strings may contain connection credentials.
        Err(Error::internal_error().data(
            "Goose could not initialize Lens HTML publication. Check Goose's extension configuration and try again.",
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn require_http(capabilities: &McpCapabilities) -> Result<(), Error> {
    if capabilities.http {
        Ok(())
    } else {
        Err(Error::invalid_params()
            .data("The Agent must support HTTP MCP for Lens HTML publication"))
    }
}

pub(crate) fn session_request(cwd: &Path, publisher: &HttpPublisher) -> NewSessionRequest {
    NewSessionRequest::new(cwd).mcp_servers(vec![McpServer::Http(
        McpServerHttp::new("lens_output", publisher.endpoint_url()).headers(vec![HttpHeader::new(
            "Authorization",
            publisher.authorization_header(),
        )]),
    )])
}

/// Structured execution metadata, separate from user-editable prompt prose.
/// A turn UUID is an output correlation receipt, never a bearer credential.
pub(crate) fn publication_context(turn_id: Uuid, embedded: bool) -> ContentBlock {
    let json = serde_json::json!({
        "kind": "lens_output_publication",
        "schema_version": 1,
        "turn_id": turn_id,
    })
    .to_string();
    if embedded {
        ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(
                TextResourceContents::new(json, format!("lens://output-publication/{turn_id}"))
                    .mime_type("application/json"),
            ),
        ))
    } else {
        ContentBlock::Text(TextContent::new(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_goose_publisher_failure_blocks_startup() {
        use serde_json::json;
        for meta in [
            json!(null),
            json!({}),
            json!({"extensionResults":[{"name":"lens_output","success":true}]}),
            json!({"extensionResults":[{"name":"other","success":false}]}),
            json!({"extensionResults":"invalid"}),
            json!({"extensionResults":[null,{"name":"lens_output","success":"false"}]}),
        ] {
            assert!(require_no_publisher_failure(Some("goose"), meta.as_object()).is_ok());
        }
        for results in [
            json!([{"name":"lens_output","success":false,"error":"SECRET"}]),
            json!([{"name":"lens_output","success":true},{"name":"other","success":true},{"name":"lens_output","success":false}]),
        ] {
            let meta = json!({"extensionResults":results});
            let error = require_no_publisher_failure(Some("goose"), meta.as_object()).unwrap_err();
            assert!(!format!("{error:?}").contains("SECRET"));
            for agent in [None, Some("other")] {
                assert!(require_no_publisher_failure(agent, meta.as_object()).is_ok());
            }
        }
    }
    #[test]
    fn http_support_is_explicit() {
        assert!(require_http(&McpCapabilities::default()).is_err());
        assert!(require_http(&McpCapabilities::default().http(true)).is_ok());
    }
    #[test]
    fn publication_context_preserves_identity_in_both_encodings() {
        let id = Uuid::new_v4();
        for embedded in [true, false] {
            let block = publication_context(id, embedded);
            let json = match block {
                ContentBlock::Text(text) => text.text,
                ContentBlock::Resource(resource) => match resource.resource {
                    EmbeddedResourceResource::TextResourceContents(text) => text.text,
                    _ => panic!("expected text resource"),
                },
                _ => panic!("expected publication context"),
            };
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["turn_id"], id.to_string());
            assert_eq!(value["kind"], "lens_output_publication");
        }
    }
    #[tokio::test]
    async fn registration_uses_only_owned_http_endpoint() {
        let publisher = HttpPublisher::start().await.unwrap();
        let request = session_request(Path::new("/workspace"), &publisher);
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["mcpServers"][0]["type"], "http");
        assert_eq!(json["mcpServers"][0]["url"], publisher.endpoint_url());
        assert_eq!(json["mcpServers"][0]["headers"][0]["name"], "Authorization");
        assert_eq!(
            json["mcpServers"][0]["headers"][0]["value"],
            publisher.authorization_header()
        );
        assert!(json["mcpServers"][0].get("command").is_none());
    }
}
