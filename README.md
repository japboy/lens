<p align="center">
  <img src="apps/desktop/src-tauri/icons/icon-macos.svg" alt="Lens app icon" width="128" height="128">
</p>

<h1 align="center">Lens</h1>

Lens is a menu-bar-first application that reads the Accessibility Trees of selected macOS application windows, captures AX-identified image regions when present, and transforms that structured multimodal information through an ACP agent selected by the user.

The current implementation accepts one to four windows, canonicalizes their identities, and creates one Accessibility source document per target without flattening provenance. `AXImage` nodes retain links to target-provenanced bounded PNG attachments captured from their screen regions with ScreenCaptureKit and sent as ACP image blocks; no OCR step converts them to text. A target-local whole-window bitmap is used only when no usable Accessibility document can be normalized for that target. Target-set extraction and Agent input have explicit group limits. It does not handle audio or live source synchronization. Every newly confirmed target-set selection applies its deterministic Lens window frame once; later updates for the same selection preserve user geometry.

## Product terminology

Interpretation is Lens's user-facing concept: understanding selected information through the user's context and reconstructing it into a useful form, including summaries, explanations, and structural changes. The default output tab is Interpretation; a continuously refreshed, self-contained result is a Living Interpretation, not an Agent-chat transcript.

- `Representation` names the complete output artifact (`LensRepresentation`).
- `Transformation` names the internal processing pipeline; `Transforming` and `transform_*` keep that technical meaning.
- `Agent` names the ACP execution role. Interpreter is not a replacement protocol or type name.
- `Translation` is reserved for actual conversion between human languages.

`pnpm run check:terminology` checks product source paths and content against this boundary. Intentional language-conversion terminology requires a documented, exact allowance in that policy. Historical experiment identities and evidence remain unchanged.

## Requirements

- Apple silicon Mac with macOS 15.2 or later
- Xcode Command Line Tools
- [mise](https://mise.jdx.dev/) 2026.8.10 or later
- Accessibility permission
- Screen Recording permission for AX-linked image regions and the explicit whole-window fallback; text-only Accessibility remains usable independently when image capture is unavailable

macOS 15.2 is the minimum because it is the first version that provides the public `includedWindows` API needed to deterministically obtain the selected `SCWindow` from the `SCContentFilter` returned by the native picker.

## Development

### Icon resources

The [canonical monochrome SVG](apps/desktop/src-tauri/icons/icon.svg) is the single source for the Lens mark. The application uses a derived smoked-glass treatment; the menu bar uses the undecorated template symbol. Run `pnpm run generate:icons` after changing the source or its generation contract. `pnpm run check:icons` verifies every generated resource and also runs in portable CI. See [icon resources](apps/desktop/src-tauri/icons/README.md) for appearance and platform boundaries.

### Build and run

The repository uses peer Cargo and pnpm workspaces. `apps/desktop` owns the private
`desktop` JavaScript package and nested Tauri Cargo package; `packages/domain`,
`packages/use-case`, `packages/port-platform`, and `packages/adapter-platform-macos`
own shared Rust responsibilities. Root `repo` owns repository tooling. Package-role
names do not change the `Lens` product, `lens` executable or `lens_lib` Rust library.
The two development pnpm members share the root lock; embedded Agent runtime
manifests, policies and locks remain independent under `apps/desktop/src-tauri/agent-runtime`.

```sh
mise trust
mise install --locked
pnpm install --frozen-lockfile
mise exec -- hk install --mise
mise run verify
mise run desktop:dev
```

mise installs the checksummed development toolchain from `mise.lock`: Node.js `24.19.0` (the current LTS major), pnpm `11.22.0`, Rust `1.98.0`, cargo-deny `0.20.2`, and hk `1.56.1`. JavaScript packages resolve through [Takumi Guard](https://shisho.dev/%64ocs/ja/t/guard/quickstart/npm/) and pnpm enforces a three-day release quarantine, no-downgrade trust policy, blocked exotic transitive sources, frozen integrity locks, and an explicit package-name build allowlist. Renovate proposes weekly updates using the same quarantine. Patch, pin, GitHub Action digest, and npm lock-file maintenance updates merge automatically only after the required Code Quality check passes; minor, major, toolchain, and managed-runtime updates require manual review.

hk installs repository-local `pre-commit` and `commit-msg` hooks through mise. The pre-commit hook checks staged frontend and configuration files with Oxfmt and Oxlint and activates Cargo formatting checks for staged Rust changes. It is check-only: it neither stashes, rewrites, nor stages files. The commit-message hook enforces Conventional Commits. Run `pnpm run fix` explicitly to apply available Oxfmt, Oxlint, and rustfmt fixes. Full tests and dependency checks remain authoritative in the required Code Quality workflow rather than a state-dependent pre-push hook.

The root TypeScript solution declares shared strict, no-emit checks and explicitly references separate application and Node.js tooling projects. Repository policy scripts are strict TypeScript executed through Node.js 24's stable native type stripping; the Node.js project admits only erasable syntax and type-checks policy scripts and tool configuration without exposing Node.js globals to browser source code. No third-party TypeScript execution loader is required.

Root mise tasks own cross-language orchestration; root pnpm scripts are compatibility
delegates, not a second task graph. `mise run verify:portable` currently runs repository
and frontend checks; `mise run verify:native` independently runs native Cargo checks,
Clippy, tests and the locked dependency audit. `mise run verify` orders portable work
before the native Cargo writer chain. Independent policy/frontend branches may run
concurrently. Verification tasks never skip based on filesystem freshness.

`mise run frontend:build` checks all TypeScript projects before producing app assets.
`mise run desktop:dev` and `mise run desktop:build` invoke app-local Tauri commands;
Tauri's own pre-build/pre-dev hooks call only the frontend tasks in that package.
`pnpm --filter desktop run build` remains an independent frontend entry point.
The task model uses mise's [declared dependencies and ordering constraints](https://mise.jdx.dev/tasks/task-configuration.html).

The application bundle does not contain Claude, Codex, their ACP adapters, or a separate Node.js runtime. The first explicit selection of an Agent installs only that Agent's approved runtime under the application-local data directory. Lens validates the official ACP Registry package identity, downloads an application-managed pnpm from Takumi Guard, verifies its pinned SHA-512 digest and executable-file hashes, and installs from an embedded exact pnpm lock with an empty lifecycle-script allowlist. It also verifies the pinned Node.js `24.19.0` archive SHA-256 digest and the Developer ID team and signing identifier of Node and the provider executable before launch. Managed installation does not require mise, Corepack, or pnpm in the user environment and never falls back to `PATH`, globally installed packages, or `npx @latest`.

To produce a debug application bundle:

```sh
mise run desktop:build -- --debug
```

The application creates no ordinary WebView window at startup. Left-clicking the `Lens` menu-bar item immediately opens the native single-window Lens Target picker when an authenticated Agent is selected. After the first selection, the entire narrow frameless Preview HUD window slides in once from beyond the right edge of its display and settles at the right-center work-area inset. Confirming the selection or removing its final item slides the whole HUD back beyond that edge before destruction. Adding or removing a non-final item never replays the entrance: AppKit instead animates the existing HUD's count-derived size and centered position as one frame transition while the affected preview card uses a paired opacity-and-horizontal-motion transition. The native and content transitions are skipped when macOS Reduce Motion is enabled. The HUD shows bounded still previews and quiet icon actions to add another window, remove an item, or confirm the one-to-four-window collection. Every addition uses a fresh system picker; confirmation alone starts extraction and Agent transformation. When target selection is unavailable, the template icon is rendered at half opacity and the click has no effect. Right-clicking the item opens this native menu. When Accessibility permission is missing at first launch, the application invokes the standard macOS permission onboarding flow.

- `Select Lens Targets…`
- `AI Agent` (Claude / Codex)
- `Working Directory: <absolute path>…`
- `Settings…`
- `Quit Lens`

An Agent receives a check mark only after Lens installs and verifies its managed runtime and then verifies its existing authentication. Codex is verified by ACP session creation; Claude is first verified by the adapter's official CLI authentication-status command because Claude Agent ACP does not reject unauthenticated session creation. Clicking an unauthenticated Agent opens the adapter-owned authentication flow in Settings. `Select Lens Targets…` remains disabled until an authenticated Agent is selected. Clicking the Working Directory item opens the native macOS folder picker and updates the path shown in the menu.

Settings provides Agent selection, adapter-owned authentication, reauthentication, sign-out, an editable Agent Prompt, Working Directory, and Accessibility permission controls. The Agent Prompt controls the response transformation while Lens keeps its source-data boundary and safety instructions fixed. Reauthentication, sign-out, and restoring the built-in prompt first show a native confirmation dialog. Settings contains no Lens Target button; Lens Target selection is a menu-bar action. Its persistent sidebar is protected by a 715-point minimum width, its content-derived maximum width is 920 points, and its detail pane preserves vertical scrolling when variable content requires it; native maximize/zoom remains available within the same constraints. Independently scrolling sidebar navigation keeps a concise application-wide Lens status pinned below it, while Settings operation feedback remains in the detail destination that owns the operation.

For one selected window, the Lens window is sized to 80% of the canonical target and centered in that target. For multiple selected windows, it is centered on the primary screen with a 10:16 portrait aspect ratio and a height equal to 80% of that screen. A changed target-set `selection_id` reapplies the applicable size and position to an existing Lens window before showing and focusing it. Reusing the window with the same selection preserves its user-managed frame, so later Accessibility-context or Agent-output revisions cannot move or resize it. The window uses normal stacking and remains movable and resizable without following later target or screen changes. Its transparent WebView surface reveals macOS's active native `HudWindow` material during standard presentation so background colors and structure remain recognizable while text is visibly blurred; increased-contrast and reduced-transparency modes become opaque. Interpretation is the default tab. Ordered ACP text and image chunks remain typed output blocks. Text deltas render through an append-only Markdown DOM with a stream cursor and reader-aware auto-scroll; terminal text settles once as sanitized GitHub Flavored Markdown. Completed `mermaid` code fences are then rendered from the bundled Mermaid runtime with strict security, bounded input, deterministic IDs, and system-aware light/dark themes; invalid diagrams remain visible as source code. PNG, JPEG, GIF, WebP, and AVIF output renders inline, while unsupported content remains explicit. Source shows the Agent-bound images one at a time in their exact input order with target provenance, followed by a pretty-printed view of the same compact, versioned multi-source `LensInput`; Diagnostics shows per-source AX and aggregate media capture metrics. The native newline-joined Accessibility text is diagnostic compatibility data only. Loading uses only bundled assets.

## Security Boundary

- Lens has the single Tauri identity `com.github.japboy.lens`. Settings, managed runtimes, and authentication cache are created under the canonical Lens identity; local state from another application identity is not read or migrated, and managed runtimes are installed and verified again when needed.
- Lens stores only the last successfully selected agent, Agent Prompt, and working directory. Authentication status is verified at runtime and is not persisted.
- Managed Agent runtimes are provider-specific, versioned application data. Interrupted installs remain in staging and are never selected; an invalid existing install is quarantined before replacement.
- It does not read or write OAuth tokens, API keys, or Claude/Codex credential files.
- Accessibility and image-region extraction are limited to the windows explicitly selected by the user. Bounded PNG payloads are kept only in operation-scoped memory and sent to the selected ACP agent as image input; they are not persisted or exposed in the WebView snapshot or JavaScript state. The Lens overlay can resolve only the current operation's exact media URIs through a read-only, non-caching custom protocol while an input image is selected in Source.
- Lens exposes no coding client capabilities to the ACP agent.
- Authentication methods and logout come from the agent adapter. Codex browser authentication and Claude terminal authentication start adapter-owned flows; ACP `logout` delegates sign-out to the selected adapter. Lens stores no tokens, API keys, or credential files.

## Validation

`pnpm run verify` runs product-identity, product-terminology, publication-boundary, and repository-language policies, Oxfmt and Oxlint checks, the root TypeScript solution across its separate browser and Node.js tooling projects, the Lit frontend build and Vitest suite, locked Rust check/format/Clippy/unit tests, and cargo-deny advisory/license/source checks. Oxfmt intentionally excludes generated Tauri schemas and semantic validation fixtures.

An on-device managed-runtime install and verification can be run independently for each Agent:

```sh
LENS_VALIDATE_RUNTIME=claude pnpm run tauri dev
LENS_VALIDATE_RUNTIME=codex pnpm run tauri dev
```

On-device PoC validation covers native Safari window selection, Accessibility extraction beyond the visible viewport, LensInput generation, managed Codex ACP transformation, Lit overlay display, cancellation, and menu-bar residency. Claude validation on this machine covers adapter startup and the explicit authentication-required flow; authenticated transformation remains a follow-up on a machine with an eligible Claude account.

A debug-only rich-output fixture replays ACP message and tool-call notifications through the output reducer, then exercises the bundled Tauri WebView with ordered Markdown, a completed tool's inline PNG, trailing Markdown, and an operation-scoped Source input-image preview without requiring Agent authentication, Accessibility permission, or Screen Recording permission:

```sh
LENS_VALIDATE_RICH_OUTPUT=1 pnpm exec tauri dev --no-watch
```

Set `LENS_VALIDATE_ACP_UPDATES` to an absolute path containing a JSON array of ACP `session/update` notification envelopes to replay local evidence instead of the committed synthetic fixture. The file is read only by this debug validation path. Keep private session data outside the repository. A headless PNG replay check reports the normalized images' dimensions, byte lengths, and SHA-256 hashes without printing their content:

```sh
LENS_VALIDATE_ACP_UPDATES=/absolute/path/session-updates.json \
  cargo test --locked -p lens replay_local_acp_images -- --ignored --nocapture
```

Official references: [Tauri application identifier configuration](https://v2.tauri.app/reference/config/#identifier), [Tauri 2.11.5 application path resolver source](https://docs.rs/crate/tauri/2.11.5/source/src/path/desktop.rs), [ACP Registry](https://agentclientprotocol.com/get-started/registry), [pnpm supply-chain security settings](https://pnpm.io/settings#minimumreleaseage), [Takumi Guard npm compatibility](https://shisho.dev/%64ocs/ja/t/guard/quickstart/npm/), [Node.js 24.19.0 distribution](https://nodejs.org/dist/v24.19.0/), and [Apple's Code Signing Requirement Language](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/RequirementLang/RequirementLang.html).
