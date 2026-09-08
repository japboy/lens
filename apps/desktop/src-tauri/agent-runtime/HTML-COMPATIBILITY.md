# HTML publisher compatibility

Lens pins and verifies the complete upstream file SHA-256 before applying these
small patches, then verifies the complete patched file SHA-256. Missing or
ambiguous anchors fail closed. Install record schema 4 binds the helper source
digest; existing unpatched caches are quarantined and reinstalled through the
existing managed-runtime installation flow. No user-owned runtime is patched.

- Codex ACP 1.6.2: the live MCP completion and replay paths forward the actual
  successful `lens_output` / `publish_html` result's HTML resource into ACP
  `content`. All other tools and raw text remain unchanged.
- Claude ACP 0.70.0: the SDK-facing adapter receives normalized tool results.
  The compatibility branch reconstructs the owned publisher's echo result from
  the correlated cached `mcp__lens_output__publish_html` input after success.
  It does **not** assume that the SDK preserved the original resource envelope.
  The URI is deterministic from the tool-use ID rather than the server UUID.
  This relies on the publisher's contract to return its valid HTML unchanged.

Both branches reject errors, empty or oversized HTML, and unrelated tools. The
Claude branch additionally rejects missing/mismatched cache IDs and extra input
fields. Neither branch parses HTML from arbitrary text, JSON strings, or links.
The existing renderer remains responsible for safe HTML/CSS rendering.

Upstream source entry points:

- <https://github.com/agentclientprotocol/codex-acp/tree/v1.6.2/src>
- <https://github.com/agentclientprotocol/claude-agent-acp/blob/v0.70.0/src/tools.ts>
- <https://github.com/agentclientprotocol/claude-agent-acp/blob/v0.70.0/src/acp-agent.ts>

Helper suites run in the normal repository Vitest suite through
`scripts/output-forwarding.test.ts`. Rust tests cover patch policy and fail-closed
anchor handling. Actual pinned distribution copies were also patched in isolated
temporary directories with full input/output checksum verification.

On 2026-09-08, a real Codex session (`gpt-5.6-sol`, read-only mode) with the
patched adapter and compiled Rust publisher delivered a `text/html` resource in
ACP tool-call content. A subsequent ordinary arithmetic question in the same
session returned only text and no resource. WebKit rendered the received HTML
in the existing Hero and exercised its expand/return controls. This combines a
live protocol probe and a component harness; it is not a native-app end-to-end
test of window selection through agent invocation.

Claude live validation requires an authenticated account. The isolated probe
returned `Authentication required`; synthetic calls against the actual patched
0.70.0 `tools.js` verified success reconstruction, error fallback, and unrelated
tool fallback. Live Claude validation is explicitly deferred, not claimed passed.
