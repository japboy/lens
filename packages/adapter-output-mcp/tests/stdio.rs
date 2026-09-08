use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    responses: Receiver<String>,
}

impl Server {
    fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lens-output-mcp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || loop {
            let mut line = String::new();
            match stdout.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            stdin: child.stdin.take(),
            responses,
            child,
        }
    }

    fn send(&mut self, value: Value) {
        writeln!(self.stdin.as_mut().unwrap(), "{value}").unwrap();
    }

    fn receive(&mut self) -> Value {
        let line = self
            .responses
            .recv_timeout(Duration::from_secs(5))
            .expect("MCP response deadline");
        serde_json::from_str(&line).unwrap()
    }

    fn initialize(&mut self) {
        self.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"lens-test","version":"1"}}}));
        let response = self.receive();
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(response["result"]["serverInfo"]["name"], "lens-output-mcp");
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }

    fn close_and_wait(&mut self) {
        self.stdin.take();
        self.wait_for_exit();
    }

    fn wait_for_exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "MCP child did not terminate on stdin EOF"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn real_stdio_handshake_tool_schema_publish_errors_and_eof() {
    let mut server = Server::new();
    server.initialize();
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let list = server.receive();
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(list["result"]["tools"][0]["name"], "publish_html");
    assert!(list["result"]["tools"][0]["description"]
        .as_str()
        .unwrap()
        .contains("Ordinary answers can remain text"));
    assert_eq!(
        list["result"]["tools"][0]["annotations"]["openWorldHint"],
        false
    );
    assert_eq!(
        list["result"]["tools"][0]["inputSchema"]["additionalProperties"],
        false
    );
    server.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"publish_html","arguments":{"html":"<h1>Hello World</h1>"}}}));
    let result = server.receive();
    assert_eq!(result["result"]["content"][0]["type"], "resource");
    assert_eq!(
        result["result"]["content"][0]["resource"]["mimeType"],
        "text/html"
    );
    assert_eq!(
        result["result"]["content"][0]["resource"]["text"],
        "<h1>Hello World</h1>"
    );
    for (id, arguments) in [
        (4, json!({"html":""})),
        (5, json!({"html":"x","path":"/tmp/x"})),
        (
            6,
            json!({"html":"x".repeat(adapter_output_mcp::MAX_HTML_BYTES + 1)}),
        ),
    ] {
        server.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"publish_html","arguments":arguments}}));
        let error = server.receive();
        assert!(
            error.get("error").is_some() || error["result"]["isError"] == true,
            "{error}"
        );
        if let Some(content) = error["result"]["content"].as_array() {
            assert!(content.iter().all(|item| item["type"] == "text"));
        }
    }
    server.close_and_wait();
}

#[test]
fn eof_before_handshake_terminates_process() {
    Server::new().close_and_wait();
}

#[test]
fn maximum_html_survives_wire_and_repeated_calls_have_distinct_identity() {
    let mut server = Server::new();
    server.initialize();
    let html = "a".repeat(adapter_output_mcp::MAX_HTML_BYTES);
    let mut uris = Vec::new();
    for id in [2, 3] {
        server.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"publish_html","arguments":{"html":html}}}));
        let result = server.receive();
        assert_eq!(result["result"]["content"][0]["resource"]["text"], html);
        uris.push(result["result"]["content"][0]["resource"]["uri"].clone());
    }
    assert_ne!(uris[0], uris[1]);
    server.close_and_wait();
}

#[test]
fn oversized_unterminated_frame_ends_server_without_waiting_for_eof() {
    let mut server = Server::new();
    server.initialize();
    // It may close while the final bytes are being written; either result is OK.
    let _ = server
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&vec![b'a'; adapter_output_mcp::MAX_FRAME_BYTES + 1]);
    server.wait_for_exit();
}

#[test]
fn malformed_line_does_not_poison_next_request_and_partial_eof_exits() {
    let mut server = Server::new();
    server.initialize();
    server
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{not-json}\n")
        .unwrap();
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    assert_eq!(server.receive()["id"], 2);
    server
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"unfinished\":")
        .unwrap();
    server.close_and_wait();
}
