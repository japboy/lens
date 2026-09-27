# Issue #131: Antigravity feasibility results

Experiment date: 2026-09-27 (JST). Lens baseline commit:
`531c605fe214fe7fcdbc2df2f516a8ac5280dc77`.

## Transition to the product implementation (2026-09-27)

The External profile in this report describes the experimental launch path. After
this experiment, the user confirmed the final product direction: **Lens manages
the official Antigravity adapter**, alongside Claude and Codex. The measurements
and their original scope below remain historical evidence. Product implementation
adds official artifact download and verification, managed installation and updates,
a distinct Agent identity, backward-compatible settings, and further validation of
production session processing and UI. This independent experiment alone does not
complete acceptance of managed installation or product UI.

## Managed implementation follow-up

Lens now includes Google Antigravity as a managed Agent, with an independent
settings record and history identity. It downloads the official signed archive,
verifies both executable signatures and hashes, and reuses the existing managed
installation, candidate/current selection, and process lease lifecycle.

The final installer test passed in 44.28 seconds: fresh download and extraction,
Google signature verification, tampered-record and tampered-helper rejection,
real ACP startup admission, activation, restoration, and an unchanged same-version
update. It installed neither Node nor pnpm. See [managed runtime evidence](evidence/managed-runtime.json).

The production persistent actor test passed in 66.17 seconds with cached OAuth,
a saved mode, one exact `allow_once` publication approval, the real HTTP publisher,
and native representation commit. See [managed actor evidence](evidence/managed-actor.json).
This used a disposable managed installation and Tauri MockRuntime. Visible WebView
rendering and installation through the user's actual app-data UI remain unverified.
Image, Japanese-language, and cancellation results below are from the earlier
independent ACP experiment, not a rerun through the managed actor.

## Conclusion

**Using Google's official `antigravity-acp` 1.2.1 through an existing External ACP
profile is technically feasible, with the conditions below.** The real binary
passed ACP v1, HTTP MCP, image input, consecutive Japanese responses, cancellation
during generation, and a subsequent turn. Actual HTML reached Lens's unchanged
`adapter-output-mcp::HttpPublisher` and was accepted for the requested turn. These
experiments did not reproduce a defect requiring a community repair proxy.

This proves protocol compatibility and use of the production MCP library through
independent experimental processes. It does not establish end-to-end success
through Lens profile saving, permission dialogs, native commit, or WebView display.
The provider omitted `updatedAt` from session listings. Externally created sessions
whose activity Lens has never observed cannot enter the current Recent Sessions
view without that timestamp.

## Environment and identity

| Item                         | Observed value                                                                              |
| ---------------------------- | ------------------------------------------------------------------------------------------- |
| OS                           | macOS 26.7 (25G229)                                                                         |
| CPU                          | arm64                                                                                       |
| Adapter                      | Google Antigravity ACP 1.2.1                                                                |
| Registry                     | Registry main also specified 1.2.1 at the time                                              |
| Source                       | `https://dl.google.com/agy-extensions/releases/macos/agy-acp-server-1.2.1-darwin-arm64.zip` |
| Archive SHA-256              | `0fab9938812e6b32b3b543e65e4f3a0025ceef755413db13542d9a9b81ea803c`                          |
| `agy_acp_server.par` SHA-256 | `c93c86c0f505fcdf8b13c695bed26d306141ef5446189d591397074d324db34e`                          |
| Model                        | Adapter default `gemini-3.8-flash-high`                                                     |
| Mode                         | `default`; no switch to Auto Edit or YOLO                                                   |
| Authentication               | Official `oauth-personal`, authorized and completed by the user                             |
| Working directory / input    | Empty temporary directory per run / short synthetic Japanese text, HTML, and red/blue PNGs  |

The model name is the observed `session/new` `currentValue`, not an assessment of
model quality or availability for other accounts. Binary identity is recorded in
[runtime.json](evidence/runtime.json); negotiation is under `tests.initialize` in
each evidence file.

## Measurements

Japanese strings below use JSON Unicode escapes to preserve exact values in this
repository's Latin-only documentation format.

| Test                                                          | Result                                                            | Evidence and scope                                                                                                          |
| ------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Unauthenticated startup                                       | Correct authentication requirement                                | `-32000 Authentication required`; [probe-unauthenticated.json](evidence/probe-unauthenticated.json)                         |
| Official Google login                                         | Passed                                                            | After user interaction, `authenticate` returned `{}`; a subsequent independent process created a session                    |
| ACP / HTTP / image capability                                 | Passed                                                            | protocolVersion=1, HTTP=true, image=true; deserialization also passed using Lens's pinned SDK schema                        |
| Session creation without MCP, equivalent to selection probing | Passed                                                            | [probe-authenticated.json](evidence/probe-authenticated.json)                                                               |
| Production HTTP MCP connection and HTML acceptance            | Passed                                                            | Publication in [focused.json](evidence/focused.json) contains the exact expected HTML and UUID                              |
| Japanese response                                             | Passed                                                            | Exact `\u6625\u306e\u7a7a\u306f\u9752\u3044\u3002`; [live-denied.json](evidence/live-denied.json)                           |
| Red / blue PNG input                                          | Passed                                                            | Same question without a color hint; 8x8 RGB PNGs received `\u8d64` / `\u9752`, respectively                                 |
| Immediate cancellation and next turn                          | Passed                                                            | `stopReason=cancelled`, then the next turn returned `\u518d\u958b\u6210\u529f`                                              |
| Cancellation after streaming began and next turn              | Passed                                                            | Cancel sent after an agent message chunk; `cancelled`, then `end_turn` and the expected next-turn text                      |
| Rejected publish_html                                         | Termination confirmed                                             | Initial harness comparison error caused `reject_once`; no publication, prompt terminated, subsequent turn succeeded         |
| session/list                                                  | Response and target session confirmed; product limitation remains | Target appeared in same-process and independent-process listings, without `updatedAt`                                       |
| Independent history load                                      | Partially demonstrated                                            | New ACP process, no client effects or MCP; load succeeded, 14 updates, known response marker replayed, zero client requests |
| session/resume                                                | API response confirmed                                            | Result returned; no new turn after resume was tested                                                                        |

The HTTP path used no mock server. Requests passed the existing production
publisher's `Authorization`, Host, and turn UUID checks, and `tools/call` actually
registered HTML. This demonstrates header and tool-call interoperability beyond
self-reported capabilities. Complete HTTP requests were not captured, so this
report does not independently measure the raw `MCP-Protocol-Version` value or each
initialize / tools/list wire frame.

The accepted HTML was exactly the following JSON-decoded string (the evidence
contains the same value):

```json
"<article><h1>\u5b9f\u8a3c\u5b9f\u9a13</h1><p>\u308a\u3093\u3054\u306f\u8d64\u3044\u3002</p></article>"
```

The successful HTML prompt terminated after 6.147 seconds. Japanese took 2.694
seconds, the red PNG 3.875 seconds, and the blue PNG 3.363 seconds. Each is one
small-input observation, not a performance SLA or an estimate of repeated-run
reliability.

## Experimental setup corrections

1. Python ZIP extraction did not preserve executable mode, causing Permission
   denied for the bundled `localharness_external`. Marking both binaries executable
   resolved this setup problem; it was not an adapter compatibility defect.
2. The first permission check required equality with the entire rawInput object
   and rejected the adapter's additional `arguments` wrapper. The corrected check
   requires exact `_meta.mcp` server/tool identity and expected HTML / turn UUID in
   both the flat and nested arguments, then selects a unique `allow_once` option.
   The retry succeeded. The initially denied attempt remains in `live-denied.json`.

Permission was limited to this single synthetic tool payload. This is not a
sandbox for operations the Agent performs internally without requesting client
assistance. Prompts prohibited unrelated filesystem, terminal, and browser work,
and those client capabilities were not provided.

## Specific history limitation

`session/list` returned `sessionId`, `cwd`, and `title`, but no `updatedAt`. An
independent process confirmed this, so the observation is not limited to a
same-process cache. Lens orders Recent by a provider timestamp or locally observed
activity. Externally created sessions with neither are excluded. Sessions created
or used through Lens have local activity and are not excluded solely for this
provider omission.

Synchronization time must not replace activity time, since that would invent the
historical order. Formal support for importing external sessions requires either
upstream timestamps or a product decision to present history with unknown activity
times separately. Successful load demonstrates replay by a known session ID,
separately from list discovery. Fourteen updates and one matching marker do not
prove complete transcript fidelity or absence of provider-internal side effects.

## Reproduction

Requires Python 3, a Rust toolchain, macOS arm64, network access, and an eligible
Google account. Run from the repository root. Official binaries are stored in a
temporary directory outside the repository.

```sh
python3 experiments/antigravity-acp/download.py
# Prints /.../lens-antigravity-131-XXXX; substitute that path below.
python3 experiments/antigravity-acp/authenticate.py --binary /.../lens-antigravity-131-XXXX/runtime/agy_acp_server.par
python3 experiments/antigravity-acp/publisher/build.py --target-dir /tmp/lens-antigravity-publisher-target
python3 experiments/antigravity-acp/probe.py --binary /.../lens-antigravity-131-XXXX/runtime/agy_acp_server.par --phase probe --output /tmp/antigravity-probe.json
python3 experiments/antigravity-acp/probe.py --binary /.../lens-antigravity-131-XXXX/runtime/agy_acp_server.par --publisher /tmp/lens-antigravity-publisher-target/debug/lens-antigravity-publisher-probe --phase live --output /tmp/antigravity-live.json
python3 experiments/antigravity-acp/probe.py --binary /.../lens-antigravity-131-XXXX/runtime/agy_acp_server.par --publisher /tmp/lens-antigravity-publisher-target/debug/lens-antigravity-publisher-probe --phase focused --output /tmp/antigravity-focused.json
```

`authenticate.py` explicitly initiates OAuth; the adapter stores its own provider
credentials. The observed authentication used the same JSON-RPC
`authenticate(methodId=oauth-personal)` from a temporary experimental controller.
The saved helper reproduces that flow. Its syntax was checked; an additional
interactive login was not repeated. `probe.py` reuses authentication and never
starts login automatically after authentication failure. The corrected live phase
permits HTML, so it does not reproduce the initial `live-denied.json` rejection.

The experimental External profile candidate uses absolute
`command=/.../runtime/agy_acp_server.par` and `args=[]`. Keep
`localharness_external` in the same directory. Do not save an experimental temporary
path as a permanent setting; an external runtime would need a durable location.
This experiment did not modify Lens settings. This launch recipe is historical;
the final product target is the managed installation described above.

## Conditions remaining after the independent experiment

1. Validate profile connection, permission responses, native HTML commit, and
   WebView display through actual Lens UI. The final product path is managed.
2. Validate real LensInput, multiple sources, embedded context, and large PNGs,
   including image formats, size limits, and errors.
3. Exercise consecutive HTML publications, MCP registration after load/resume, and
   rejection of delayed output from obsolete turns through the product lifecycle.
4. Decide the product treatment of missing activity timestamps for external history.
5. Test ordinary tool approval/rejection, process termination and reconnection,
   expired authentication, and account-specific availability limits.
6. Evaluate managed installation, updates, signatures, and proprietary distribution
   terms as separate design and validation work.

## Primary sources and implementation

- [Official Registry pinned definition](https://github.com/agentclientprotocol/registry/blob/d3e4c258cc125f3baa76c66d71fdb8dd9ffb2025/antigravity-acp/agent.json)
- [Google's Zed integration](https://antigravity.google/docs/ide/extensions/zed)
- [ACP session setup](https://agentclientprotocol.com/protocol/v1/session-setup)
- [Lens HTTP publisher source](https://github.com/japboy/lens/blob/531c605fe214fe7fcdbc2df2f516a8ac5280dc77/packages/adapter-output-mcp/src/lib.rs)
- [Lens session MCP registration](https://github.com/japboy/lens/blob/531c605fe214fe7fcdbc2df2f516a8ac5280dc77/apps/desktop/src-tauri/src/output_mcp.rs)
- [Lens history activity and admission](https://github.com/japboy/lens/blob/531c605fe214fe7fcdbc2df2f516a8ac5280dc77/apps/desktop/src-tauri/src/session_view.rs)

The official adapter is proprietary. This report does not claim inspection of its
internal source code. Compatibility conclusions rest on saved responses from the
actual binary and the inspectable Lens implementation.
