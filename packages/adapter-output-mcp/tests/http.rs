use adapter_output_mcp::{HttpPublisher, MAX_FRAME_BYTES, MAX_HTML_BYTES};
use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};
use uuid::Uuid;

struct Mcp {
    client: Client,
    url: String,
    authorization: String,
    session: Option<String>,
}
impl Mcp {
    async fn connect(server: &HttpPublisher) -> Self {
        let mut mcp = Self {
            client: Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
            url: server.endpoint_url().into(),
            authorization: server.authorization_header().into(),
            session: None,
        };
        let response = mcp.post(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"lens-test","version":"1"}
        }})).await;
        assert_eq!(response.status(), StatusCode::OK);
        mcp.session = response
            .headers()
            .get("mcp-session-id")
            .map(|v| v.to_str().unwrap().to_owned());
        assert!(body(response).await["result"]["serverInfo"].is_object());
        let response = mcp
            .post(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        assert!(response.status().is_success());
        mcp
    }
    async fn post(&self, value: Value) -> Response {
        let mut request = self
            .client
            .post(&self.url)
            .header("authorization", &self.authorization)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-03-26")
            .json(&value);
        if let Some(id) = &self.session {
            request = request.header("mcp-session-id", id);
        }
        request.send().await.unwrap()
    }
    async fn call(&self, args: Value) -> Value {
        body(self.post(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"publish_html","arguments":args}})).await).await
    }
}
async fn body(response: Response) -> Value {
    let raw = response.text().await.unwrap();
    serde_json::from_str(&raw).unwrap_or_else(|_| {
        raw.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .find_map(|data| serde_json::from_str(data).ok())
            .expect("SSE JSON data")
    })
}
fn rejected(value: &Value) -> bool {
    value.get("error").is_some() || value["result"]["isError"] == true
}

#[tokio::test]
async fn publishes_exact_html_once_and_identical_retry_has_host_generated_identity() {
    let server = HttpPublisher::start().await.unwrap();
    let mcp = Mcp::connect(&server).await;
    let listed = body(
        mcp.post(json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}))
            .await,
    )
    .await;
    let tool = &listed["result"]["tools"][0];
    assert_eq!(tool["name"], "publish_html");
    assert_eq!(tool["annotations"]["readOnlyHint"], true);
    assert_eq!(tool["annotations"]["destructiveHint"], false);
    assert_eq!(tool["annotations"]["openWorldHint"], false);
    assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    let id = Uuid::new_v4();
    let turn = server.begin_turn(id).unwrap();
    assert!(server.begin_turn(Uuid::new_v4()).is_err());
    let html = "<style>h1{color:red}</style><h1>\u{4e16}\u{754c}</h1><svg></svg>";
    let first = mcp.call(json!({"turn_id":id,"html":html})).await;
    assert!(!rejected(&first), "{first}");
    let receipt: Value =
        serde_json::from_str(first["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let retry = mcp.call(json!({"turn_id":id,"html":html})).await;
    assert_eq!(first, retry);
    assert!(rejected(
        &mcp.call(json!({"turn_id":id,"html":"different"})).await
    ));
    let publication = turn.finish().unwrap().unwrap();
    assert_eq!(publication.html, html);
    assert_eq!(receipt["publication_id"], publication.id.to_string());
    assert!(rejected(&mcp.call(json!({"turn_id":id,"html":html})).await));
}

#[tokio::test]
async fn empty_text_turn_and_cancelled_turn_do_not_leak_into_next_turn() {
    let server = HttpPublisher::start().await.unwrap();
    let mcp = Mcp::connect(&server).await;
    assert!(server.begin_turn(Uuid::nil()).is_err());
    let empty = server.begin_turn(Uuid::new_v4()).unwrap();
    assert!(empty.finish().unwrap().is_none());
    let old = Uuid::new_v4();
    let cancelled = server.begin_turn(old).unwrap();
    assert!(!rejected(
        &mcp.call(json!({"turn_id":old,"html":"old"})).await
    ));
    drop(cancelled);
    assert!(rejected(
        &mcp.call(json!({"turn_id":old,"html":"old"})).await
    ));
    let next_id = Uuid::new_v4();
    let next = server.begin_turn(next_id).unwrap();
    assert!(rejected(
        &mcp.call(json!({"turn_id":old,"html":"old"})).await
    ));
    assert!(next.finish().unwrap().is_none());
}

#[tokio::test]
async fn validates_schema_utf8_budget_and_frame_budget() {
    let server = HttpPublisher::start().await.unwrap();
    let mcp = Mcp::connect(&server).await;
    let id = Uuid::new_v4();
    let turn = server.begin_turn(id).unwrap();
    for args in [
        json!({"turn_id":id,"html":""}),
        json!({"turn_id":id,"html":" \n\t"}),
        json!({"turn_id":"invalid","html":"valid"}),
        json!({"html":"valid"}),
        json!({"turn_id":id,"html":"valid","publication_id":"untrusted"}),
        json!({"turn_id":id,"html":"a".repeat(MAX_HTML_BYTES + 1)}),
        json!({"turn_id":id,"html":"\u{754c}".repeat(MAX_HTML_BYTES / 3 + 1)}),
    ] {
        assert!(rejected(&mcp.call(args).await));
    }
    assert!(!rejected(
        &mcp.call(json!({"turn_id":id,"html":"a".repeat(MAX_HTML_BYTES)}))
            .await
    ));
    assert_eq!(turn.finish().unwrap().unwrap().html.len(), MAX_HTML_BYTES);
    let oversized = mcp.post(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"publish_html","arguments":{"turn_id":id,"html":"a".repeat(MAX_FRAME_BYTES)}}})).await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn rejects_missing_wrong_or_duplicate_credentials_and_foreign_host_origin() {
    let server = HttpPublisher::start().await.unwrap();
    let client = Client::builder().no_proxy().build().unwrap();
    for authorization in [None, Some("Bearer invalid")] {
        let mut request = client.get(server.endpoint_url());
        if let Some(value) = authorization {
            request = request.header("authorization", value);
        }
        assert_eq!(
            request.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let request = || {
        client
            .get(server.endpoint_url())
            .header("authorization", server.authorization_header())
    };
    assert_eq!(
        request()
            .header("authorization", server.authorization_header())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for host in ["attacker.invalid", "localhost:1234", "127.0.0.1:1"] {
        assert_eq!(
            request()
                .header("host", host)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    for origin in ["https://evil.example", "null", "http://127.0.0.1:1"] {
        assert_eq!(
            request()
                .header("origin", origin)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    let same_origin = server.endpoint_url().trim_end_matches("/mcp");
    assert_ne!(
        request()
            .header("origin", same_origin)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    for path in ["/control", "/state"] {
        assert_eq!(
            client
                .get(format!("{same_origin}{path}"))
                .header("authorization", server.authorization_header())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn publisher_drop_revokes_turn_and_closes_listener_and_partial_connections() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = HttpPublisher::start().await.unwrap();
    let _mcp = Mcp::connect(&server).await;
    let turn = server.begin_turn(Uuid::new_v4()).unwrap();
    let authority = server
        .endpoint_url()
        .trim_start_matches("http://")
        .trim_end_matches("/mcp")
        .to_owned();
    let mut partial = tokio::net::TcpStream::connect(&authority).await.unwrap();
    partial.write_all(b"POST /mcp HTTP/1.1\r\n").await.unwrap();
    tokio::task::yield_now().await;
    drop(server);
    assert!(turn.finish().is_err());
    let closed = tokio::time::timeout(std::time::Duration::from_secs(2), partial.read(&mut [0; 1]))
        .await
        .unwrap();
    assert!(matches!(closed, Ok(0) | Err(_)));
    assert!(tokio::net::TcpStream::connect(authority).await.is_err());
}

#[tokio::test]
async fn rejects_oversized_chunked_body_without_content_length() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let server = HttpPublisher::start().await.unwrap();
    let authority = server
        .endpoint_url()
        .trim_start_matches("http://")
        .trim_end_matches("/mcp");
    let mut stream = tokio::net::TcpStream::connect(authority).await.unwrap();
    let header = format!("POST /mcp HTTP/1.1\r\nHost: {authority}\r\nAuthorization: {}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n", server.authorization_header());
    stream.write_all(header.as_bytes()).await.unwrap();
    let payload = format!(
        "{:x}\r\n{}\r\n0\r\n\r\n",
        MAX_FRAME_BYTES + 1,
        "a".repeat(MAX_FRAME_BYTES + 1)
    );
    // An early rejection may close the connection before all bytes are written.
    let _ = stream.write_all(payload.as_bytes()).await;
    let mut response = [0u8; 1024];
    let count = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        stream.read(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(std::str::from_utf8(&response[..count])
        .unwrap()
        .starts_with("HTTP/1.1 413"));
}

#[tokio::test]
async fn isolated_sessions_cannot_publish_to_each_others_turns() {
    let first = HttpPublisher::start().await.unwrap();
    let second = HttpPublisher::start().await.unwrap();
    assert_ne!(first.authorization_header(), second.authorization_header());
    let first_mcp = Mcp::connect(&first).await;
    let second_mcp = Mcp::connect(&second).await;
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let first_turn = first.begin_turn(first_id).unwrap();
    let second_turn = second.begin_turn(second_id).unwrap();
    assert!(rejected(
        &first_mcp
            .call(json!({"turn_id":second_id,"html":"cross-session"}))
            .await
    ));
    assert!(rejected(
        &second_mcp
            .call(json!({"turn_id":first_id,"html":"cross-session"}))
            .await
    ));
    assert!(!rejected(
        &first_mcp
            .call(json!({"turn_id":first_id,"html":"first"}))
            .await
    ));
    drop(first);
    assert!(first_turn.finish().is_err());
    assert!(!rejected(
        &second_mcp
            .call(json!({"turn_id":second_id,"html":"second"}))
            .await
    ));
    assert_eq!(second_turn.finish().unwrap().unwrap().html, "second");
}
