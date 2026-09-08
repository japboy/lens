# Lens HTML publication MCP server

`lens-output-mcp` is a standalone, stdio-only process. The ACP agent starts it
from Lens's session-scoped MCP configuration. It does not require Node.js,
Python, a GUI, persistent MCP configuration, network access, or file access.

The sole tool, `publish_html`, accepts `{ "html": "<h1>Hello</h1>" }` and returns
one MCP embedded resource with `mimeType: "text/html"` and an opaque
`urn:lens:html:<UUID>` URI. The URI identifies the returned content; it is not a
file path or a fetchable endpoint. The HTML bytes are preserved. Blank input,
unknown arguments, and HTML larger than 512 KiB are rejected. Each successful
invocation has a distinct resource identity; tools/list declares that the tool
has no external mutations or open-world access, but is not idempotent.

The tool does not sanitize or execute HTML. The Lens renderer owns its restricted
HTML/CSS policy. The agent decides whether to call the tool: no automatic call,
retry, or missing-artifact failure is imposed here.

MCP transport, initialization, schema discovery, and call dispatch use the
[official Rust MCP SDK](https://docs.rs/rmcp/3.2.0/rmcp/) with only server, macros,
and stdio features enabled. A bounded reader limits incoming newline-delimited
JSON to `512 KiB * 6 + 16 KiB` before SDK deserialization, allowing JSON escaping
plus protocol overhead while bounding untrusted frame allocation. An overlong
frame closes the transport. Closing stdin ends the process. Stdout is exclusively
MCP; startup failures go to stderr.

Run `cargo test -p adapter-output-mcp` for input-limit tests and a real child
process handshake / tools/list / tools/call / EOF test. This validates the MCP
boundary, not an ACP adapter's preservation of typed resource content.
