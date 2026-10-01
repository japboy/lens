# Product MCP Apps native acceptance

The debug-only `mcp_apps_validation` module seeds public synthetic Lens input, changes configuration only in memory, and invokes `agent::transform_current`. It uses the normal managed Codex session actor, MCP broker, output publication, native Overlay and frontend Host. The debug driver seeds the explicit Codex selection precondition in memory before admission; that selection flag is not connection or authentication evidence. The normal actor must still resolve, connect and authenticate the actual installed runtime. The driver does not seed a Checking candidate, because the production candidate confirmation persists the entire configuration. It does not simulate an Agent, mount an experimental browser Host, evaluate UI scripts, or perform App clicks. The procedure below is followed by explicitly scoped records from the actual native product run.

## Launch contract

Use the actual bundled debug Tauri product build, with the frontend built into the custom-scheme native application. The root operator owns the launched process, source fixture servers, and CUA interactions. Do not change existing approvals, authentication, working directory or persistent settings. The managed Codex runtime must already be installed and authenticated. Normal provider session history and Lens session/artifact retention may be written through the production path; record the created operation/artifact identities for owned cleanup.

For the original official basic App:

```text
LENS_VALIDATE_MCP_APPS=external
LENS_VALIDATE_MCP_APPS_URL=http://127.0.0.1:43159/mcp/basic
```

The URL must be a credential-free loopback HTTP source. The existing protocol experiment source server exposes the original upstream `basic-server-vanillajs` MCP factory here. Only its MCP endpoint is reused; its experimental browser Host is not product evidence. Source pin and hashes are in `experiments/mcp-apps-generic/SOURCE.md` and `upstream/SHA256.json`.

For Lens's bundled fallback, launch a separate owned debug process with:

```text
LENS_VALIDATE_MCP_APPS=bundled
```

Do not combine this hook with the other `LENS_VALIDATE_*` initializers. Each launch creates a fresh public fixture operation, then retains one live Agent session for App follow-up.

## Required observable acceptance

1. External: actual Codex calls the source's advertised `get-time`. Product Overlay displays the original App from original metadata/resource/result. Operate its Get Server Time, then its Send Message control. The App request must remain bound to the original source. App message acknowledgment creates a trusted Lens draft; it must not start an Agent turn by itself. Click the actual Lens Send action. Verify an actual second Codex response, with the same session SHA256 and a new run ID.
2. Bundled: actual Codex calls `render_html`. Overlay displays the generated HTML in the same product Host. Click Set context 10, then Set context 73, then Ask Agent. Observe successful App operations and the trusted draft. Click Lens Send. Verify an actual response explaining 73 in Japanese; the old context must not replace the latest one. Verify the same live session SHA256 with a new run ID.
3. Ordinary lifecycle: close the displayed App and reopen its retained artifact. Replace the display and exercise stale lease rejection through the production authority tests. Observe native responsiveness during these normal operations, frame/listener/bridge cleanup and cancellation of the previous authority. A renderer process's immediate exit and survival under non-yielding JavaScript are not acceptance requirements.
4. Replacement parity: actual Codex uses the bundled renderer with body TeX and a public HTTP(S) anchor. Observe bundled math rendering, authored JavaScript interaction, a trusted link proposal and explicit default-browser opening without an Agent turn. Provider history must recover validated built-in HTML as inert source text, preserving the existing Conversation behavior. Confirm the former publisher is absent from the session registration and tool inventory.
5. Native isolation: an independent App document cannot access the parent/top DOM or native invocation key/IPC. Inspect only presence/absence booleans, never print key values. Check real source/origin restrictions and source-bound tool visibility. A general WebKit handler object alone is not evidence of privileged native access.

`LENS_MCP_APPS_STATE` records actual stage, run ID, session SHA256, App artifact IDs, output SHA256 and retained response count. These state observations are separate from root's native screenshots and actual UI operation evidence. They do not certify a rendered UI or a click. No input or source credential is emitted by the hook.

## Relevant production contracts

- `apps/desktop/src-tauri/src/agent.rs`: `transform_current`, `current_transform_input`, `run_persistent_session_actor`.
- `apps/desktop/src-tauri/src/agent_runtime.rs`: `resolve_for_session` leases an already published managed installation; installation belongs to explicit user actions.
- `apps/desktop/src-tauri/src/mcp_apps.rs`: retained source artifacts, display leases, App requests, trusted draft submission and lease revocation.
- `packages/adapter-mcp-apps-server/src/apps.rs`: actual source catalog/resource/result capture and the bundled `render_html` tool.
- `apps/desktop/src/components/lens-mcp-app.ts`: product Host display, SDK bridge and user controls.

Record any actual failure at its concrete layer (source transport, Agent tool execution, resource capture, native display, App operation, trusted submission or lifecycle). Do not promote previous standalone browser/prototype results into native product evidence.

## Recorded external App native acceptance — 2026-09-30

The root operator ran the bundled debug product on macOS 26.7.1, with Tauri 2.12, Wry 0.57 and MCP Apps SDK 2.0.3. CUA observed and operated the actual Overlay at `tauri://localhost`. The source was the pinned original official basic App described above. This run used the real managed Codex runtime and production actor, broker, publication and display paths with the public fixture and memory-only configuration described in the launch contract.

The identifiers below are corroborated by the bounded native state records in `/tmp/lens-mcp-apps-external-native.log`. UI behavior and window closure were observed by the root operator through CUA; native state records alone do not prove those observations.

| Record                                              | Identifier or SHA256                                                            |
| --------------------------------------------------- | ------------------------------------------------------------------------------- |
| Operation                                           | `233afd90-e7e0-47ea-b169-bc84701d0048`                                          |
| Same live Agent session SHA256, all three responses | `dd71d104fd7908dd462894e0712c7c227e73f9dc237181f79bb5c4ddf735199e`              |
| Initial run                                         | `75f7b447-4ea9-4e19-b635-ab2c5a2a638e`                                          |
| First trusted follow-up run                         | `1b1bb461-cf6b-4f9d-b1ec-b847cee78f06`                                          |
| Second trusted follow-up run                        | `af623091-c2f1-491c-b7ff-c0bb26348dda`                                          |
| Original retained App artifact                      | `f062b563-bfa0-4f52-bb48-c32f244fc0aa`                                          |
| Initial display lease / generation                  | `6f741389-b41f-441b-998d-e762763b1f94` / `ba0cc67d-f2dc-438a-aec3-9f3a96416a9e` |
| Reopened display lease / generation                 | `3c5fce6c-cf1a-4b18-8361-53e1d93385cf` / `0da0d196-1404-4a4e-95d7-b06c5f800916` |

Actual Codex called the original `get-time` tool and Lens displayed the original official App and its captured result. Operating Get Server Time updated the App through the originating MCP source. Operating Send Message created a native Lens draft while the retained response count stayed at one. Clicking trusted Lens Send produced the second actual Codex response, explaining the requested UTC-to-Japan conversion in Japanese, with the same live session SHA256 and a new run ID.

Closing the App removed its frames and native display leases; the initial display listener was absent. Reopening the retained artifact created the new lease and generation above, retained the original captured result, and left the response count at two without starting another Agent turn. The reopened retained App remained usable after the first follow-up: a second App message created another Lens draft, and trusted Send produced the third actual Codex response in the same session. That response explicitly reported an empty App context and did not claim to choose between the fixture's two context candidates.

Closing the App again left zero native display leases and removed the reopened display listener. The ordinary Lens close action removed the Overlay window; CUA reported no available Lens windows. The root operator then stopped the owned debug process and external source process.

This records the external original App, source-bound interaction, two trusted follow-ups in one live Agent session, and ordinary close/reopen/cleanup in the native product. It does not certify the bundled HTML fallback, latest-context replacement, or the generated document's native-key/DOM isolation diagnostics; those require separate actual native observations. No App HTML, native key value, credential or raw provider session identifier is included here.

## Recorded bundled HTML native acceptance — partial

The second bundled attempt used the same macOS/Tauri/Wry/SDK environment above. Actual Codex generated HTML through the bundled fallback and CUA observed it in the native Overlay. The generated document's own diagnostics reported parent DOM access `false`, top DOM access `false`, own `__TAURI_INTERNALS__` presence `false`, invoke-key property presence `false`, and own `ipc` presence `false`. WebKit IPC handler presence was `true`; the presence of that general handler is not evidence of native command authority. These observations establish the displayed document's reported access/presence behavior, not a successful native invocation attempt.

| Record                            | Identifier or SHA256                                               |
| --------------------------------- | ------------------------------------------------------------------ |
| Second attempt operation          | `46eab68b-8445-4ee5-83b6-22b65ee8d90a`                             |
| Second attempt session SHA256     | `35fa04200f603957e082b2c3cb81f106003b4dfc297ef6f19a8717877c376d44` |
| Displayed generated HTML artifact | `aad4b8c4-5cdb-4b97-b8a3-e21104bb5a1b`                             |

CUA clicks did not produce the expected interaction, including on a parent Lens tab; the user reported that manual tab interaction did respond. The manual tab remount then exposed a Loading state with no native lease. A connected-only remount regression fix was implemented and tested after this observation. SDK text/structured capability advertisement and CSP advertisement were also corrected. Those code/test results must not be substituted for actual generated-App context selection or trusted follow-up observations.

A third run used the latest main integration at `bff1d1c`. The bounded state records in `/tmp/lens-mcp-apps-bundled-native-final.log` corroborate actual Agent completion, one retained response, the generated App artifact, and one active native display lease. CUA explicitly reported that the Mac was locked, preventing screen acquisition and actual UI operations. The user said they could not unlock it at that time. At the end of that attempt, its rendered UI, context 10-to-73 replacement, trusted Send follow-up and normal Quit were unverified; the later run below records the resumed acceptance separately.

| Record                            | Identifier or SHA256                                                            |
| --------------------------------- | ------------------------------------------------------------------------------- |
| Third attempt operation           | `696f189e-bdb4-45d5-a136-a143e5a1a6e9`                                          |
| Third attempt run                 | `5acc2dda-6e71-43fd-904c-4ad0cf13c7fb`                                          |
| Third attempt session SHA256      | `4a88be26ede6551c6d575178c5d1e37f1da4295dff7e2c404cead79a702da059`              |
| Generated HTML artifact           | `8f3753e3-b3fc-4819-8913-a8b786bae75f`                                          |
| Native display lease / generation | `a2e23473-7f50-4a0b-97fb-971faab9e4a4` / `2fea0b71-1861-4318-820a-b8d159afb253` |

The root operator stopped the third owned process with Ctrl-C and cleaned only the two known public fixture artifact directories. This forced process stop does not prove normal Quit cleanup. An explicit normal-Quit cleanup change passed eleven native tests and the latest bundled build, but actual normal Quit on that build was not observed. At that time actual UI acceptance was paused and PR creation was deferred pending the remaining observations.

The first bundled attempt, operation `b75c1dca-d2cb-4507-9193-c26cee9b43c9`, session SHA256 `d5ed32e4e35bc2aa8d1d8f9392404ce69a69edd2474950868232a905ba6c908c`, was superseded into Selecting before a generated App was confirmed. Its cause is unknown. A limited audit of the exact owned session's single relevant 121-byte tool input found only an available-tool inventory lookup; it contained no selection, stop, shell or GUI operation. The input SHA256 was `db5a4b112dd7fd54f60362d62b6128ff5a9f53abea6eac3d2b6e0e8f1cf60086`. The initial native log was overwritten, so that limited audit cannot establish the transition's cause. This interrupted attempt is separate from the second attempt's confirmed native rendering and the third attempt's completed backend state.

Outstanding at the end of that attempt were generated App Set context 10, then Set context 73, then Ask Agent; trusted Lens Send; an actual Japanese response using latest context 73 with the same live session SHA256; ordinary remount/replacement after the connected-only fix; and normal Quit cleanup. The following resumed run completed these observations. No App HTML, native key value, credential or raw provider session identifier is recorded here.

## Final bundled HTML native acceptance — 2026-10-01

After the user unlocked the Mac, the root operator resumed acceptance with the actual bundled debug product at `e99375f`, using the same public synthetic fixture and real managed Codex production path. The binary SHA256 was `b7095cf165850717c4f162102355f972719b10ca08bf425fdc441f58a502797c`. CUA operated the native Lens Overlay and its generated App; this was not a standalone browser Host. The state transitions below were independently corroborated from `/tmp/lens-mcp-apps-bundled-native-resume.log`; actual rendering, clicks and Japanese response text were observed by the root operator through CUA/native accessibility. The environment remained the macOS/Tauri/Wry/SDK configuration recorded above.

| Record                                              | Identifier or SHA256                                                            |
| --------------------------------------------------- | ------------------------------------------------------------------------------- |
| Operation                                           | `d32289d1-94de-4b10-9bf3-1b0df3d06d97`                                          |
| Same live Agent session SHA256, all three responses | `02854e5f9eb5dc532949af7a7dba46f8ad8d7d3f261960ff637ef8f26e5c6bfb`              |
| Initial run                                         | `b3c46993-8e4d-4366-8428-46e7c0629343`                                          |
| Latest-context trusted follow-up run                | `ff1e0f4c-a51f-4829-a113-371f982c6221`                                          |
| Empty-context trusted follow-up run                 | `c1eb70af-8675-4559-92a0-ee9266135063`                                          |
| Original generated HTML artifact                    | `4f55848c-34fd-4af1-b8ec-0eb922971b5f`                                          |
| Reopened lease / generation                         | `87941913-eb53-46ff-a9bb-c93dfb2d3be9` / `277c40dc-8f8d-49cb-b235-14c577a9a971` |
| Lease / generation after Conversation tab cycle     | `a3e83ebe-6ca1-4906-9cd1-7db8af27b07b` / `ee224422-b048-428b-8a1c-2fc561e8c2f6` |
| Empty-context message draft                         | `d7691dd3-051c-4e7d-8edc-2b8945faa549`                                          |

Actual Codex generated the bundled HTML App and completed the first response. CUA operated Set context 10, then Set context 73, then Ask Agent; all three standard App RPC operations completed. The resulting trusted draft left the response count at one, kept the initial run ID and had not been submitted. Only clicking trusted Lens Send started the second run. The completed Japanese response used `selected_value` 73, with response count two and the same live session SHA256. This confirms latest-context replacement and separates App message acknowledgment from Agent turn admission.

Closing the App removed its frame and left zero native display leases; its listener on port 53043 refused connections. Reopen retained the original artifact and created the new lease/generation above on port 53093. The response count stayed at two and the run ID stayed unchanged, so display reopening did not start an Agent turn. The App's operation results were empty after remount. Switching to Conversation removed the lease and closed port 53093. Returning to Interpretation rendered the original HTML again on port 53103 without becoming stuck at Loading, using another new lease/generation.

After that tab-cycle remount, CUA operated Ask Agent without setting new context. It created the draft above; trusted Lens Send started the third run in the same live session. The third response completed without error, with response count three. The actual Japanese reply reported that the current App context was empty, `selected_value` could not be confirmed, and the previous context's 73 could not be carried forward. This verifies that resetting display context is reflected in the follow-up sent to the existing Agent session, rather than leaving the old value authoritative.

For normal Quit, the root operator first confirmed that the live App listener on port 53103 accepted a connection and the owned temporary artifact file and directory existed. The operator then clicked the native Lens menu's Quit Lens action. The owned process (PTY 76345) exited with code zero without Ctrl-C or another forced stop. Its accessibility surface disappeared, the listener on port 53103 refused connections, and both the owned `lens-mcp-apps-Ic4kiS` directory and its `4f55848c-34fd-4af1-b8ec-0eb922971b5f.json` artifact file no longer existed. Two stale menu-target guards during navigation were resolved by obtaining fresh complete accessibility state before the successful Quit action; they were not treated as successful clicks.

Together with the external App run above, these observations complete the scoped Codex/macOS native acceptance: original external App interaction, generated fallback, trusted same-session follow-up, latest-context replacement, context reset, responsive close/reopen/tab remount and normal Quit cleanup. They do not establish survival under non-yielding JavaScript, immediate renderer-process exit, exhaustive native-command denial or behavior on other providers/platforms. Earlier interrupted or locked attempts remain historical records, not successful observations. No App HTML, native key value, credential or raw provider session identifier is included.

## Replacement and package extraction qualification — 2026-10-01

The completed native observations above precede the confirmed removal of the old
static publisher and the subsequent package extraction. They do not establish
parity acceptance for these later changes.

A replacement debug bundle connected the actual managed Codex Agent, received a
generated built-in App and displayed it in the native Overlay. The run was stopped
before math, anchor opening, App input or trusted follow-up acceptance when final
authority review identified delayed first-arrival ambiguity across Agent runs. The
bundled renderer currently accepts HTML without a caller-supplied run identity and
associates arrival with the active run. The user requested structural improvement
first and consideration of this concern afterward. No successful fresh parity or
turn-admission claim is made from that interrupted run.

At the initial extraction checkpoint, the generic browser Host/transport/proxy
was placed in `adapter-mcp-apps-web`, HTML/math and browser/Node resource generation in
`adapter-rich-content-web`, and pure App input/authority rules plus the session
document reducer in `usecase`. Desktop retained native locks, IPC, physical
sessions, storage and effect dispatch. Tool schemas and
admission semantics are unchanged during this structural step.

Fresh automated frontend and repository gates passed: shared packages 39 tests,
Desktop 664 tests and repository 582 tests. Types, formatting, lint, explicit
workspace boundaries and generated WebView admission passed. Package sources are
sealed into the generation hash, and local workspace dependency resolution remains
inside the copied generation. The release fixture uses an isolated fixed commit of
current workspace declarations without weakening production release admission.

The complete macOS gate passed against the final sealed generation: 607 Rust tests
passed with 19 existing opt-in tests ignored, plus workspace Clippy, dependency
audit, documentation tests and development/release compilation. Exact package-owned
resources are consumed by native composition. Actual replacement parity acceptance
remains pending the separate turn-admission decision.

## Server/Host/View naming qualification — 2026-10-01

Role names now match package and directory names: `adapter-mcp-apps-server`,
`adapter-mcp-apps-host-web` and `adapter-mcp-apps-view-html`. Shared Markdown/static
HTML math and Node resource generation remain in `adapter-rich-content-web`.
Desktop composes Host, View and shared math; View consumes the public math API.
Host has no View/rendering dependency. Server includes the inbound MCP facade,
upstream client sessions and display transport, with no Tauri dependency.

Fresh naming gates passed: repository 590 tests, Host 17, View 15, shared math 7
and Desktop 664 tests. Six Cargo and six pnpm members and 554 source paths passed
explicit boundary checks. Type checking, formatting, lint and sealed generation
passed. The complete macOS gate passed 607 Rust tests with 19 existing opt-in
ignores, Clippy, dependency audit, documentation tests and dev/release compilation.
The generation is `350ac78f0d27a8f41bdf2c5f27dbe1392b9cbe0d3a3769cdf2fbd4fa8ad8878e`.

Server implementation and four canonical runtime assets match the preceding
commit exactly; View preparation changed only the math import. Tool schemas and
turn admission are unchanged. These automated results do not establish fresh
actual Codex replacement parity, which still awaits the separate turn decision.
