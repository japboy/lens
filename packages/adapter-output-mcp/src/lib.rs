//! A stdio-only MCP endpoint for explicitly published static HTML artifacts.
//! No files, network resources, or graphical runtime are accessed here.

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ResourceContents},
    schemars, tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub const MAX_HTML_BYTES: usize = 512 * 1024;
// JSON can escape each byte as six ASCII characters. Allow protocol overhead
// while bounding a frame before the SDK allocates/deserializes the full line.
pub const MAX_FRAME_BYTES: usize = MAX_HTML_BYTES * 6 + 16 * 1024;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishHtmlInput {
    /// Complete static HTML or an HTML fragment, at most 512 KiB in UTF-8.
    pub html: String,
}

#[derive(Debug, Clone)]
pub struct HtmlPublisher;

#[tool_router]
impl HtmlPublisher {
    #[tool(
        description = "Publish a static HTML artifact for display in Lens's Hero area. Use when an HTML presentation helps fulfill the user's request. Pass the HTML itself, not a file path, URL, Markdown link, or fenced code block. The renderer supports a restricted subset of HTML and CSS; scripts, interactive forms, and external resources are not supported. This tool only returns the supplied HTML as an embedded resource; it does not write files or access the network. Ordinary answers can remain text.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub fn publish_html(
        &self,
        Parameters(input): Parameters<PublishHtmlInput>,
    ) -> Result<CallToolResult, ErrorData> {
        if input.html.trim().is_empty() || input.html.len() > MAX_HTML_BYTES {
            return Err(ErrorData::invalid_params(
                "html must be non-empty and at most 524288 UTF-8 bytes",
                None,
            ));
        }
        let resource = ResourceContents::TextResourceContents {
            uri: format!("urn:lens:html:{}", uuid::Uuid::new_v4()),
            mime_type: Some("text/html".to_owned()),
            text: input.html,
            meta: None,
        };
        Ok(CallToolResult::success(vec![ContentBlock::resource(
            resource,
        )]))
    }
}

#[tool_handler(name = "lens-output-mcp", version = "0.1.0")]
impl ServerHandler for HtmlPublisher {}

pub type ServerError = Box<dyn std::error::Error + Send + Sync>;

pub async fn run_stdio() -> Result<(), ServerError> {
    run(tokio::io::stdin(), tokio::io::stdout()).await
}

/// Serve until the agent closes its transport. Stdout is reserved for MCP.
pub async fn run<R, W>(reader: R, writer: W) -> Result<(), ServerError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let service = HtmlPublisher
        .serve((BoundedLines::new(reader), writer))
        .await?;
    service.waiting().await?;
    Ok(())
}

/// Bound newline-delimited input independently of SDK deserialization behavior.
struct BoundedLines<R> {
    inner: R,
    line_bytes: usize,
    failed: bool,
}

impl<R> BoundedLines<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            line_bytes: 0,
            failed: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for BoundedLines<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.failed {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "MCP frame too large",
            )));
        }
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let mut bytes = [0; 8192];
        let count = bytes.len().min(output.remaining());
        let mut incoming = ReadBuf::new(&mut bytes[..count]);
        match Pin::new(&mut self.inner).poll_read(cx, &mut incoming) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        for byte in incoming.filled() {
            if *byte == b'\n' {
                self.line_bytes = 0;
            } else {
                self.line_bytes += 1;
                if self.line_bytes > MAX_FRAME_BYTES {
                    self.failed = true;
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "MCP frame too large",
                    )));
                }
            }
        }
        output.put_slice(incoming.filled());
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn returns_typed_html_without_modification() {
        let html = "<h1>Hello \u{4e16}\u{754c}</h1>";
        let result = HtmlPublisher
            .publish_html(Parameters(PublishHtmlInput { html: html.into() }))
            .unwrap();
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["content"][0]["type"], "resource");
        assert_eq!(value["content"][0]["resource"]["mimeType"], "text/html");
        assert_eq!(value["content"][0]["resource"]["text"], html);
        assert!(value["content"][0]["resource"]["uri"]
            .as_str()
            .unwrap()
            .starts_with("urn:lens:html:"));
    }

    #[test]
    fn enforces_utf8_byte_limit_and_nonempty_input() {
        for html in [
            "".into(),
            " \n\t".into(),
            "a".repeat(MAX_HTML_BYTES + 1),
            "\u{754c}".repeat(MAX_HTML_BYTES / 3 + 1),
        ] {
            assert!(HtmlPublisher
                .publish_html(Parameters(PublishHtmlInput { html }))
                .is_err());
        }
        assert!(HtmlPublisher
            .publish_html(Parameters(PublishHtmlInput {
                html: "a".repeat(MAX_HTML_BYTES)
            }))
            .is_ok());
        assert!(serde_json::from_value::<PublishHtmlInput>(
            serde_json::json!({"html":"x","url":"file:///x"})
        )
        .is_err());
    }

    #[tokio::test]
    async fn limits_frames_and_resets_at_newlines() {
        let oversized = vec![b'a'; MAX_FRAME_BYTES + 1];
        let mut reader = BoundedLines::new(oversized.as_slice());
        assert_eq!(
            reader
                .read_to_end(&mut Vec::new())
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        let mut allowed = vec![b'a'; MAX_FRAME_BYTES];
        allowed.push(b'\n');
        allowed.extend(vec![b'b'; MAX_FRAME_BYTES]);
        let mut reader = BoundedLines::new(allowed.as_slice());
        let mut received = Vec::new();
        reader.read_to_end(&mut received).await.unwrap();
        assert_eq!(received, allowed);
    }
}
