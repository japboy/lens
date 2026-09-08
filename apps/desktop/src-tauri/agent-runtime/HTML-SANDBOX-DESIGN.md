# Active HTML preview: design and adoption gates

Status: **Active JavaScript deferred to [Lens #58](https://github.com/japboy/lens/issues/58).** Current delivery direction is Web Component + script-disabled iframe, using standard Wry. Production renderer unchanged.

## Current decision and upstream clarification (2026-09-08)

The active-preview proposal below is retained as research, not the current shipping plan. The current scope preserves static HTML/CSS in a real iframe document without the renderer's independent CSS grammar or presentation tag allowlist; JavaScript is disabled and external loading/navigation remain explicitly controlled. This is not a guarantee of pixel-identical rendering for JavaScript-dependent pages, missing external fonts/assets, or unsupported WebKit features.

Upstream has already addressed uninitialized iframe IPC access: [Wry #1251](https://github.com/tauri-apps/wry/pull/1251) stopped macOS subframe initialization injection, and [GHSA-57fm-592m-34r7](https://github.com/tauri-apps/tauri/security/advisories/GHSA-57fm-592m-34r7) documents the invoke-key protection. The local Tauri 2.11.5 dependency is newer than the advisory's fixed versions. [Wry #1365](https://github.com/tauri-apps/wry/pull/1365) subsequently added explicit subframe injection support; it is not an IPC rejection policy.

The valid-key experiments below intentionally bypassed the key's secrecy assumption. They do not demonstrate a bypass of upstream's existing protection. The native main-frame guard is an additional defense option, **not a proven universal prerequisite for JavaScript support**. Whether that stronger requirement warrants a fork/patch is deferred to #58. No directly matching open main-frame-only IPC rejection proposal was found in the upstream search; this is not a claim that none exists.

## Requested behavior

Keep the existing Web Component, Hero presentation, media navigation, and expand/return controls. Render generated HTML/CSS/JavaScript in a sandboxed iframe. Remove the static renderer's independent presentation allowlists rather than expanding them incrementally.

## Candidate architecture

- The trusted Web Component owns the frame lifecycle and existing overlay controls.
- The iframe uses `sandbox="allow-scripts"`, without `allow-same-origin` or navigation, popup, download, or form-submission grants.
- An in-memory preview response has its own CSP. Do not relax the parent application's script policy. `srcdoc` with an additional policy cannot relax an inherited policy.
- Default-deny automatic resource/network access; permit only explicitly supported embedded resources. HTML/CSS/JavaScript functionality and resource permissions are separate concerns.
- Messages from the frame are untrusted, validated by source window, current generation, schema, and size/rate budgets. No generic native-command bridge. An external-link request is not evidence of a real user gesture and must not automatically trigger the opener.
- Revoke stale preview resources. Retain the same frame during expand/return where possible so interactive state is not reset.

## Native WKWebView experiment (2026-09-08)

A separate disposable Swift process was used; the running Lens app was not modified or navigated. The fixture used a nonpersistent WKWebView, a custom-scheme parent and preview, response CSP, and `sandbox="allow-scripts"`. It registered a benign native handler named `ipc` to test reachability, **not a real Tauri command handler**.

Observed:

| Check | Result |
| --- | --- |
| Generated inline JavaScript | Executed |
| CSS gradient | Computed correctly |
| Parent DOM access | `SecurityError` |
| localStorage access | `SecurityError` |
| HTTPS fetch with `connect-src 'none'` | `TypeError` |
| Direct `webkit.messageHandlers.ipc.postMessage` | Reached native handler from subframe; security origin empty |
| Infinite loop in child, then evaluate parent JavaScript | Parent evaluation did not return before the six-second native watchdog deadline |

The last result is an availability warning, not a claim that all native UI hangs: the Swift main run loop remained able to print the timeout and exit. The experiment does not establish actual Tauri command execution or exhaustive network isolation. Arbitrary self-navigation and other egress paths remain unverified.

Temporary reproducible fixture: `/tmp/lens-iframe-boundary.E2Awkn/probe.swift`.
Run `xcrun swift /tmp/lens-iframe-boundary.E2Awkn/probe.swift` for the boundary probe, or append `--loop` for the bounded availability probe. It does not invoke real Lens commands. The fixture deliberately contains hostile JavaScript and must not be loaded into the production overlay.

## Dependency source audit

Local resolved dependencies examined:

- `wry 0.55.1`, `src/wkwebview/class/wry_web_view_delegate.rs:50-61`: IPC forwards the frame request URL without testing `isMainFrame` or `securityOrigin`.
- `tauri 2.11.5`, `src/webview/mod.rs:1715-1721`: registered custom protocols count as local URLs. A preview custom scheme is therefore not automatically rejected as a remote origin.
- Same file, `1746-1763`: the invoke key is checked. Native-handler reachability alone is **not** proof of command execution.
- `wry 0.55.1`, `src/wkwebview/navigation.rs:50-80`: navigation actions are passed to the navigation handler without a main-frame filter. Public higher-level callbacks expose the URL, not frame identity. Exact coverage requires native tests.

## Gates before enabling active content

1. **Native authority:** reject preview-origin/frame IPC independently of the secrecy of the invoke key, including built-in/plugin and application commands. Verify with a benign test-only command and positive/negative controls. Do not infer rejection from missing `__TAURI_INTERNALS__` alone.
2. **Availability:** decide and verify how Lens remains usable when child JavaScript never yields. An iframe is not a process/CPU isolation guarantee. A parent JavaScript timer or a stop button implemented in the same blocked web context is insufficient.
3. **Egress:** verify subframe self-navigation, redirects, forms, resource loads, nested contexts, and custom schemes at the native boundary; a CSP fetch policy is not a universal navigation policy.
4. **Integration:** verify frame CSP without weakening parent CSP; source-checked messages; input/focus/scroll preservation; stale-resource revocation; existing Hero controls.

These gates prevent claiming this architecture is ready for production. Native frame-level rejection has now been demonstrated below, but is not implemented in the shipping runtime. Availability is a separate product decision; a separate WebView is not a prerequisite for the IPC guard and does not itself prove process isolation.

## Actual Tauri IPC follow-up (2026-09-08)

Fixture: `apps/desktop/src-tauri/examples/iframe_ipc_probe.rs`. It runs a **separate Tauri application using the real Wry/WKWebView runtime**, Lens's generated capability configuration, and only a benign custom command/report handler. It does not register Lens's production commands, agent services, or stores. The loopback server binds an ephemeral port on `127.0.0.1` and exits with the fixture process.

The parent deliberately supplies the correct invocation key to each test iframe. This is a stronger test condition than ordinary untrusted content; it does **not** demonstrate that a generated page can obtain that key. No key is printed. Three command classes are tested: a benign custom command, the read-only `plugin:app|version`, and `plugin:__TAURI_CHANNEL__|fetch` without a channel ID. The last command's `missing channel id header` response proves that its handler was entered; no real channel data is requested or consumed.

### Unmodified runtime control

| Native message sender | Custom command | App version | Channel fetch handler |
| --- | --- | --- | --- |
| Trusted parent | Executed | Returned | Entered |
| Sandboxed custom-scheme iframe | Executed | Returned | Entered |
| Sandboxed loopback HTTP iframe | ACL rejection | ACL rejection | Entered |

Thus, a separate HTTP origin and remote ACL alone do not supply a complete native IPC boundary: channel fetch is explicitly exempt from ACL in Tauri 2.11.5. These results assume the deliberately supplied valid key.

### Guarded runtime experiment

The fixture replaces Wry 0.55.1's Objective-C IPC method **inside that disposable process only**, before test frames load. It rejects messages whose native `WKFrameInfo.isMainFrame` is false, before invoking the original Wry/Tauri handler. It records a rejection count, so a JavaScript timeout alone is not treated as proof of rejection.

- All six native-message iframe attempts (two origins × three commands) were rejected at the native entrypoint.
- Parent custom-command and app-version calls still succeeded.
- The guard therefore works independently of the invocation key and before the channel-fetch ACL exception.
- Runtime method replacement is an experiment, **not the proposed production implementation**. A small version-pinned Wry source patch or an opt-in upstream main-frame-only IPC policy is the auditable shipping route. Current public Tauri/Wry IPC callbacks do not expose native frame identity.

### Alternate `ipc://` transport

Both iframe origins also attempted `fetch('ipc://localhost/...')` for the same three commands with the valid key and required callback headers.

- With preview `connect-src 'none'`: all six requests failed with `TypeError: Load failed`.
- With a deliberately weakened test-only `connect-src ipc:`: all six requests received HTTP 500, `Origin header is not a valid URL`, before command execution. This matches Tauri's inability to parse the sandboxed opaque origin as a URL.
- Parent fetch calls to the benign command and app-version command succeeded (positive control), so the child failures are not merely an unavailable transport.

The final fixture validates all 18 result rows per run plus native execution/rejection counters. Reproduce from the repository root:

```sh
cargo run -p desktop --example iframe_ipc_probe
cargo run -p desktop --example iframe_ipc_probe -- --guard
cargo run -p desktop --example iframe_ipc_probe -- --guard --permit-ipc-fetch
```

All three modes must print `ASSERTIONS_PASSED=true` and exit 0. The permit flag weakens **only the disposable preview fixture**, never Lens's CSP.

### Limits and next implementation conditions

The conclusion is **feasible with an additional native guard**, not "iframe sandbox alone is sufficient". Keep `allow-same-origin` absent, deny preview connections, and deny navigation into trusted application pages. Navigation/redirect/nested-context bypass tests and the actual pinned dependency patch remain integration work. This proof is macOS-specific; no cross-platform result is claimed.

One early fixture run aborted because its test instrumentation used an incorrect Objective-C class name. The macOS Problem Reporter displayed `iframe_ipc_probe`, not Lens. The name was corrected, subsequent runs passed, and setup failure now exits normally instead of unwinding across AppKit. The running Lens process was not replaced or terminated.

## References

- [MDN: iframe sandbox](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/iframe#sandbox)
- [MDN: Content Security Policy](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy)
- [MDN: postMessage](https://developer.mozilla.org/en-US/docs/Web/API/Window/postMessage)
- [Tauri: capabilities and platform caveats](https://v2.tauri.app/security/capabilities/)
