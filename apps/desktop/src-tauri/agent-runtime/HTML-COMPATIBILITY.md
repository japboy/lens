# HTML publisher compatibility

Lens pins and verifies the complete upstream file SHA-256 before applying these
small patches, then verifies the complete patched file SHA-256. Missing or
ambiguous anchors fail closed. Install record schema 4 binds the helper source
digest; existing unpatched caches are quarantined and reinstalled through the
existing managed-runtime installation flow. No user-owned runtime is patched.

- Codex ACP 1.10.0: the live MCP completion and replay paths forward the actual
  successful `lens_output` / `publish_html` result's HTML resource into ACP
  `content`. All other tools and raw text remain unchanged.
- Claude ACP 0.74.0: the SDK-facing adapter receives normalized tool results.
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

- <https://github.com/agentclientprotocol/codex-acp/tree/v1.10.0/src>
- <https://github.com/agentclientprotocol/claude-agent-acp/blob/v0.74.0/src/tools.ts>
- <https://github.com/agentclientprotocol/claude-agent-acp/blob/v0.74.0/src/acp-agent.ts>

Helper suites run in the normal repository Vitest suite through
`scripts/output-forwarding.test.ts`. Rust tests cover patch policy and fail-closed
anchor handling. Actual pinned distribution copies were also patched in isolated
temporary directories with full input/output checksum verification.

On 2026-09-08, an initial real Codex session (`gpt-5.6-sol`, read-only mode) with
a patched cached 1.6.2 adapter and compiled Rust publisher delivered a `text/html` resource in
ACP tool-call content. A subsequent ordinary arithmetic question in the same
session returned only text and no resource. WebKit rendered the received HTML
in the existing Hero and exercised its expand/return controls. This combines a
live protocol probe and a component harness; it is not a native-app end-to-end
test of window selection through agent invocation. It did not validate the fresh
installation path: the dependency lock already selected 1.10.0 while the Rust
policy still expected 1.6.2, so the application correctly rejected the mismatched
patch input. Claude had equivalent policy drift (0.70.0 versus locked 0.74.0).
These older probes must not be treated as verification of the current versions.

Regression tests bind each approved adapter version to its embedded package
manifest, and each signed executable's package/version to the dependency lock.
The manifest-policy check reproduced the 1.10.0/1.6.2 mismatch before the fix.
Changes to locked runtimes must also revalidate the complete upstream and patched
file digests and the signed native executables from a fresh installation; a cached
adapter or helper-only test is not evidence for that installation path.

The corrected policy was checked against fresh installs of Codex ACP 1.10.0
(Codex CLI 0.153.4) and Claude ACP 0.74.0 (Claude SDK 0.3.257), using the
checked-in frozen pnpm locks. The opt-in Rust test
`freshly_installed_adapters_pass_production_patch_and_runtime_verification`
successfully ran the production patch, complete patched-file hash checks, Node
and native executable signature checks, and adapter `--version` checks for both.
It requires explicit disposable installation roots in `LENS_PATCH_CODEX_ROOT`
and `LENS_PATCH_CLAUDE_ROOT`, plus the verified Node root in
`LENS_PATCH_NODE_ROOT`; it is not an authenticated Claude conversation test.

A subsequent authenticated Codex probe used that freshly installed, production-
patched 1.10.0 distribution and the bundled Rust publisher. It delivered one
typed HTML resource, then answered a normal arithmetic question with `4` and no
resource. This verifies the updated protocol path, not the native window picker.

Synthetic calls against the freshly patched Claude 0.74.0 `tools.js` verified
success reconstruction, error fallback, unrelated-tool fallback, and mismatched
call-ID fallback. Claude live validation still requires an authenticated account;
it is explicitly deferred, not claimed passed. The earlier 0.70.0 live probe
returned `Authentication required`.
