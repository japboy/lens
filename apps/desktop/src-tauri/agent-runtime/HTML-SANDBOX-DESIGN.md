# HTML preview: static delivery and deferred active-content research

Status: **Active JavaScript deferred to [Lens #58](https://github.com/japboy/lens/issues/58).** The current implementation direction is a script-disabled iframe owned directly by `lens-output-media`, using standard Wry without a native dependency patch.

## Current decision and upstream clarification (2026-09-08)

The active-preview proposal below is retained as research, not the current shipping plan. The current scope preserves static HTML/CSS in a real iframe document without the renderer's independent CSS grammar or presentation tag allowlist; JavaScript is disabled and external loading/navigation remain explicitly controlled. This is not a guarantee of pixel-identical rendering for JavaScript-dependent pages, missing external fonts/assets, or unsupported WebKit features.

Upstream has already addressed uninitialized iframe IPC access: [Wry #1251](https://github.com/tauri-apps/wry/pull/1251) stopped macOS subframe initialization injection, and [GHSA-57fm-592m-34r7](https://github.com/tauri-apps/tauri/security/advisories/GHSA-57fm-592m-34r7) documents the invoke-key protection. The local Tauri 2.11.5 dependency is newer than the advisory's fixed versions. [Wry #1365](https://github.com/tauri-apps/wry/pull/1365) subsequently added explicit subframe injection support; it is not an IPC rejection policy.

The valid-key experiments below intentionally bypassed the key's secrecy assumption. They do not demonstrate a bypass of upstream's existing protection. The native main-frame guard is an additional defense option, **not a proven universal prerequisite for JavaScript support**. Whether that stronger requirement warrants a fork/patch is deferred to #58. No directly matching open main-frame-only IPC rejection proposal was found in the upstream search; this is not a claim that none exists.

## Current static-preview architecture

- `lens-output-media` owns the iframe directly and retains the existing Hero media navigation and expand/return controls. There is no additional HTML wrapper component or child-to-parent script bridge.
- `prepareHtmlPreview` uses `parse5`, without a DOM or browsing context, so parsing does not initiate resource loads. It prepares a full HTML document while preserving CSS text and presentation structure; this is not byte-for-byte source preservation.
- The iframe uses `sandbox="allow-popups"`, without script, same-origin, popup-escape, download, or form-submission grants. The popup permission enables declarative anchor requests; the native handler denies creation of an application popup WebView. An injected CSP additionally denies scripts, connections, automatic external resources, frames, and objects while allowing inline styles and supported data-URL assets.
- Preparation removes active document controls, embedded browsing contexts, refresh metadata, and unsafe navigation attributes. Valid HTTP(S) anchors retain their destinations with `target="_blank"`; standard Tauri `on_new_window` validates the scheme, hands the destination to the OS default browser, and returns `NewWindowResponse::Deny`. No HTML-frame JavaScript, postMessage bridge, or Details link list is needed. CSP resource restrictions alone are not treated as a complete navigation policy.
- Self-contained static HTML/CSS, inline SVG, and data-URL images are supported. Input controls do not imply submission or JavaScript support. Missing external fonts/assets, script-dependent layouts, and browser feature differences remain fidelity limits.
- The existing 512 KiB resource limit and explicit publisher/ACP contract remain unchanged. Arbitrary text, code fences, and file links are not automatically promoted to HTML artifacts.

The experiments below concern **active JavaScript** and are retained as historical evidence, not validation of the current static renderer. Current static-preview implementation and tests must be reviewed independently. No Wry patch or JavaScript enablement is part of this delivery.

## Native static-link experiment (2026-09-08)

The separate `tests/fixtures/html-preview.html` fixture renders the actual Hero
components in Playwright WebKit. It verified CSS variables, grid, gradients,
inline SVG, native details/input controls, fragment scrolling, preserved input
across expand/return, external-image CSP rejection, and blocked form submission.
The Details backdrop is below the existing navigation controls. An automated
Escape press with focus inside the iframe did not exit fullscreen in this
WebKit run; the visible close-expanded-HTML button did exit and retained input.
Iframe key events are not forwarded to the parent. Full native-window keyboard
behavior is not claimed from this browser fixture.

The sandbox grants follow [MDN's iframe reference](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/iframe#sandbox).
Both parse and serialization use `scriptingEnabled: false`, including a
regression for escaped `noscript` markup; the serializer's default must not
reintroduce active markup while preparing a script-disabled document.

`examples/iframe_navigation_probe.rs` is a separate disposable Tauri application,
using the resolved Tauri 2.11.5 / Wry 0.55.1 runtime. It does not start Lens's
agent services or production windows. Actual accessibility-driven clicks were
used; no generated JavaScript or native method replacement was involved.

| Configuration                                                                | Observed result                                                                            |
| ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| Empty sandbox; parent `default-src 'self'`; child `default-src 'none'`       | Neither HTTPS nor opaque-scheme child link reached the navigation callback                 |
| Empty sandbox; parent additionally permits HTTPS frame navigation            | HTTPS reached `on_navigation` and was cancelled                                            |
| Empty sandbox; opaque scheme registered and permitted in parent frame policy | Opaque-scheme child link still did not reach the callback                                  |
| `allow-popups`; HTTPS anchor with `_blank`; original parent and child CSP    | `on_navigation`, followed by `on_new_window`; native `Deny` prevented an application popup |

The final `--popup --open-browser` run imported the production `html_preview.rs`
helper and initialized the real opener/dialog plugins. One click on the public
test URL `https://example.com/lens-probe` opened a new Firefox tab with that URL
and the Example Domain page. The preview remained `about:srcdoc`. Logs included
`POPUP_CAPTURED_AND_DENIED=https://example.com/lens-probe` and
`PRODUCTION_BROWSER_HANDOFF=https://example.com/lens-probe`.

A follow-up capture-only run used the production-equivalent anchor attributes
`target="_blank" rel="noopener noreferrer"`. An actual click again reached
`on_new_window` and was denied, without another OS browser handoff.

By default the fixture only captures and denies requests; `--open-browser` is an
explicit opt-in to the actual OS browser handoff. `--permit-frame-navigation`
reproduces the broader frame-policy control and is not part of production.
The navigation callback runs before the popup callback: denying the former
prevents the latter. This proves the tested macOS link path, not cross-platform
equivalence or a complete native-app capture-to-agent end-to-end flow.

## Deferred active-content behavior (#58)

Keep the existing Web Component, Hero presentation, media navigation, and expand/return controls. Render generated HTML/CSS/JavaScript in a sandboxed iframe. Remove the static renderer's independent presentation allowlists rather than expanding them incrementally.

## Deferred active-content candidate architecture

- The trusted Web Component owns the frame lifecycle and existing overlay controls.
- The iframe uses `sandbox="allow-scripts"`, without `allow-same-origin` or navigation, popup, download, or form-submission grants.
- An in-memory preview response has its own CSP. Do not relax the parent application's script policy. `srcdoc` with an additional policy cannot relax an inherited policy.
- Default-deny automatic resource/network access; permit only explicitly supported embedded resources. HTML/CSS/JavaScript functionality and resource permissions are separate concerns.
- Messages from the frame are untrusted, validated by source window, current generation, schema, and size/rate budgets. No generic native-command bridge. An external-link request is not evidence of a real user gesture and must not automatically trigger the opener.
- Revoke stale preview resources. Retain the same frame during expand/return where possible so interactive state is not reset.

## Native WKWebView experiment (2026-09-08)

A separate disposable Swift process was used; the running Lens app was not modified or navigated. The fixture used a nonpersistent WKWebView, a custom-scheme parent and preview, response CSP, and `sandbox="allow-scripts"`. It registered a benign native handler named `ipc` to test reachability, **not a real Tauri command handler**.

Observed:

| Check                                                   | Result                                                                          |
| ------------------------------------------------------- | ------------------------------------------------------------------------------- |
| Generated inline JavaScript                             | Executed                                                                        |
| CSS gradient                                            | Computed correctly                                                              |
| Parent DOM access                                       | `SecurityError`                                                                 |
| localStorage access                                     | `SecurityError`                                                                 |
| HTTPS fetch with `connect-src 'none'`                   | `TypeError`                                                                     |
| Direct `webkit.messageHandlers.ipc.postMessage`         | Reached native handler from subframe; security origin empty                     |
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

These gates concern adoption of active JavaScript, not the current script-disabled preview. Native frame-level rejection has now been demonstrated below, but is not implemented in the shipping runtime. Availability is a separate product decision; a separate WebView is not a prerequisite for the IPC guard and does not itself prove process isolation.

## Actual Tauri IPC follow-up (2026-09-08)

Fixture: `apps/desktop/src-tauri/examples/iframe_ipc_probe.rs`. It runs a **separate Tauri application using the real Wry/WKWebView runtime**, Lens's generated capability configuration, and only a benign custom command/report handler. It does not register Lens's production commands, agent services, or stores. The loopback server binds an ephemeral port on `127.0.0.1` and exits with the fixture process.

The parent deliberately supplies the correct invocation key to each test iframe. This is a stronger test condition than ordinary untrusted content; it does **not** demonstrate that a generated page can obtain that key. No key is printed. Three command classes are tested: a benign custom command, the read-only `plugin:app|version`, and `plugin:__TAURI_CHANNEL__|fetch` without a channel ID. The last command's `missing channel id header` response proves that its handler was entered; no real channel data is requested or consumed.

### Unmodified runtime control

| Native message sender          | Custom command | App version   | Channel fetch handler |
| ------------------------------ | -------------- | ------------- | --------------------- |
| Trusted parent                 | Executed       | Returned      | Entered               |
| Sandboxed custom-scheme iframe | Executed       | Returned      | Entered               |
| Sandboxed loopback HTTP iframe | ACL rejection  | ACL rejection | Entered               |

Thus, a separate HTTP origin and remote ACL alone do not supply a complete native IPC boundary: channel fetch is explicitly exempt from ACL in Tauri 2.11.5. These results assume the deliberately supplied valid key.

### Guarded runtime experiment

The fixture replaces Wry 0.55.1's Objective-C IPC method **inside that disposable process only**, before test frames load. It rejects messages whose native `WKFrameInfo.isMainFrame` is false, before invoking the original Wry/Tauri handler. It records a rejection count, so a JavaScript timeout alone is not treated as proof of rejection.

- All six native-message iframe attempts (two origins × three commands) were rejected at the native entrypoint.
- Parent custom-command and app-version calls still succeeded.
- The guard therefore works independently of the invocation key and before the channel-fetch ACL exception.
- Runtime method replacement is an experiment, **not the proposed production implementation**. If #58 adopts an invocation-key-independent native guard, a small version-pinned Wry source patch or an opt-in upstream main-frame-only IPC policy is a candidate implementation route. The current static delivery requires neither. Current public Tauri/Wry IPC callbacks do not expose native frame identity.

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

The experiment demonstrates that an additional native guard can reject the tested iframe IPC attempts independently of invocation-key secrecy; it does not establish that such a guard is universally necessary or that active HTML is ready to ship. For that deferred design, keep `allow-same-origin` absent, deny preview connections, and deny navigation into trusted application pages. Navigation/redirect/nested-context bypass tests and any adopted native guard remain #58 integration work. This proof is macOS-specific; no cross-platform result is claimed.

One early fixture run aborted because its test instrumentation used an incorrect Objective-C class name. The macOS Problem Reporter displayed `iframe_ipc_probe`, not Lens. The name was corrected, subsequent runs passed, and setup failure now exits normally instead of unwinding across AppKit. The running Lens process was not replaced or terminated.

## References

- [MDN: iframe sandbox](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/iframe#sandbox)
- [MDN: Content Security Policy](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy)
- [MDN: postMessage](https://developer.mozilla.org/en-US/docs/Web/API/Window/postMessage)
- [Tauri: capabilities and platform caveats](https://v2.tauri.app/security/capabilities/)
