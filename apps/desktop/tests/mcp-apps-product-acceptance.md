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
4. Native isolation: an independent App document cannot access the parent/top DOM or native invocation key/IPC. Inspect only presence/absence booleans, never print key values. Check real source/origin restrictions and source-bound tool visibility. A general WebKit handler object alone is not evidence of privileged native access.

`LENS_MCP_APPS_STATE` records actual stage, run ID, session SHA256, App artifact IDs, output SHA256 and retained response count. These state observations are separate from root's native screenshots and actual UI operation evidence. They do not certify a rendered UI or a click. No input or source credential is emitted by the hook.

## Relevant production contracts

- `apps/desktop/src-tauri/src/agent.rs`: `transform_current`, `current_transform_input`, `run_persistent_session_actor`.
- `apps/desktop/src-tauri/src/agent_runtime.rs`: `resolve_for_session` leases an already published managed installation; installation belongs to explicit user actions.
- `apps/desktop/src-tauri/src/mcp_apps.rs`: retained source artifacts, display leases, App requests, trusted draft submission and lease revocation.
- `packages/adapter-output-mcp/src/apps.rs`: actual source catalog/resource/result capture and the bundled `render_html` tool.
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
