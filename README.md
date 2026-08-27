# PersonalLens

PersonalLens is a menu-bar-first application that reads the Accessibility Tree exposed by another macOS application and transforms that information through an ACP agent selected by the user.

The PoC focuses on accessibility-based text extraction and ACP-based transformation. It does not handle images, OCR, audio, or real-time content translation. AXObserver-based selected-window geometry following is included.

## Requirements

- Apple silicon Mac with macOS 15.2 or later
- Xcode Command Line Tools
- [mise](https://mise.jdx.dev/) 2026.8.10 or later
- Accessibility permission

macOS 15.2 is the minimum because it is the first version that provides the public `includedWindows` API needed to deterministically obtain the selected `SCWindow` from the `SCContentFilter` returned by the native picker.

## Development

```sh
mise trust
mise install --locked
pnpm install --frozen-lockfile
mise exec -- hk install --mise
pnpm run verify
pnpm run tauri dev
```

mise installs the checksummed development toolchain from `mise.lock`: Node.js `24.19.0` (the current LTS major), pnpm `11.22.0`, Rust `1.98.0`, cargo-deny `0.20.2`, and hk `1.56.1`. JavaScript packages resolve through [Takumi Guard](https://shisho.dev/%64ocs/ja/t/guard/quickstart/npm/) and pnpm enforces a three-day release quarantine, no-downgrade trust policy, blocked exotic transitive sources, frozen integrity locks, and an explicit package-name build allowlist. Renovate proposes weekly updates using the same quarantine. Patch, pin, GitHub Action digest, and npm lock-file maintenance updates merge automatically only after the required Code Quality check passes; minor, major, toolchain, and managed-runtime updates require manual review.

hk installs repository-local `pre-commit` and `commit-msg` hooks through mise. The pre-commit hook checks staged frontend and configuration files with Oxfmt and Oxlint and activates Cargo formatting checks for staged Rust changes. It is check-only: it neither stashes, rewrites, nor stages files. The commit-message hook enforces Conventional Commits. Run `pnpm run fix` explicitly to apply available Oxfmt, Oxlint, and rustfmt fixes. Full tests and dependency checks remain authoritative in the required Code Quality workflow rather than a state-dependent pre-push hook.

The root TypeScript solution declares shared strict, no-emit checks and explicitly references separate application and Node.js tooling projects. Repository policy scripts are strict TypeScript executed through Node.js 24's stable native type stripping; the Node.js project admits only erasable syntax and type-checks policy scripts and tool configuration without exposing Node.js globals to browser source code. No third-party TypeScript execution loader is required.

The application bundle does not contain Claude, Codex, their ACP adapters, or a separate Node.js runtime. The first explicit selection of an Agent installs only that Agent's approved runtime under the application-local data directory. PersonalLens validates the official ACP Registry package identity, downloads an application-managed pnpm from Takumi Guard, verifies its pinned SHA-512 digest and executable-file hashes, and installs from an embedded exact pnpm lock with an empty lifecycle-script allowlist. It also verifies the pinned Node.js `24.19.0` archive SHA-256 digest and the Developer ID team and signing identifier of Node and the provider executable before launch. Managed installation does not require mise, Corepack, or pnpm in the user environment and never falls back to `PATH`, globally installed packages, or `npx @latest`.

To produce a debug application bundle:

```sh
pnpm run tauri build --debug
```

The application creates no ordinary WebView window at startup. Left-clicking the `Lens` menu-bar item immediately opens the native Lens Target picker when an authenticated Agent is selected. When target selection is unavailable, the template icon is rendered at half opacity and the click has no effect. Right-clicking the item opens this native menu. When Accessibility permission is missing at first launch, the application invokes the standard macOS permission onboarding flow.

- `Select Lens Target…`
- `AI Agent` (Claude / Codex)
- `Working Directory: <absolute path>…`
- `Settings…`
- `Quit PersonalLens`

An Agent receives a check mark only after PersonalLens installs and verifies its managed runtime and then verifies its existing authentication. Codex is verified by ACP session creation; Claude is first verified by the adapter's official CLI authentication-status command because Claude Agent ACP does not reject unauthenticated session creation. Clicking an unauthenticated Agent opens the adapter-owned authentication flow in Settings. `Select Lens Target…` remains disabled until an authenticated Agent is selected. Clicking the Working Directory item opens the native macOS folder picker and updates the path shown in the menu.

Settings provides Agent selection, adapter-owned authentication, reauthentication, sign-out, an editable Agent Prompt, Working Directory, and Accessibility permission controls. The Agent Prompt controls the response transformation while PersonalLens keeps its source-data boundary and safety instructions fixed. Reauthentication, sign-out, and restoring the built-in prompt first show a native confirmation dialog. Settings contains no Lens Target button; Lens Target selection is a menu-bar action. Its preferred size shows all default content without scrolling, while its monitor-aware height cap and compact layout keep it within an HD work area and preserve scrolling when variable content requires it.

The Lens overlay opens centered at 80% of the selected window's width and height, follows its move/resize lifecycle, and uses a lightly translucent native background material. Translation is the default tab. ACP text deltas render through an append-only Markdown DOM with a stream cursor and reader-aware auto-scroll; terminal output settles once as sanitized GitHub Flavored Markdown. Source keeps the extracted Accessibility text and diagnostics available without flashing them as the Agent result. Loading uses only bundled assets.

## Security Boundary

- PersonalLens stores only the last successfully selected agent, Agent Prompt, and working directory. Authentication status is verified at runtime and is not persisted.
- Managed Agent runtimes are provider-specific, versioned application data. Interrupted installs remain in staging and are never selected; an invalid existing install is quarantined before replacement.
- It does not read or write OAuth tokens, API keys, or Claude/Codex credential files.
- Accessibility extraction is limited to a snapshot of the target explicitly selected by the user.
- PersonalLens exposes no coding client capabilities to the ACP agent.
- Authentication methods and logout come from the agent adapter. Codex browser authentication and Claude terminal authentication start adapter-owned flows; ACP `logout` delegates sign-out to the selected adapter. PersonalLens stores no tokens, API keys, or credential files.

## Validation

`pnpm run verify` runs publication-boundary and repository-language policies, Oxfmt and Oxlint checks, the root TypeScript solution across its separate browser and Node.js tooling projects, the Lit frontend build and Vitest suite, locked Rust check/format/Clippy/unit tests, and cargo-deny advisory/license/source checks. Oxfmt intentionally excludes generated Tauri schemas and semantic validation fixtures.

An on-device managed-runtime install and verification can be run independently for each Agent:

```sh
PERSONAL_LENS_VALIDATE_RUNTIME=claude pnpm run tauri dev
PERSONAL_LENS_VALIDATE_RUNTIME=codex pnpm run tauri dev
```

On-device PoC validation covers native Safari window selection, Accessibility extraction beyond the visible viewport, LensInput generation, managed Codex ACP transformation, Lit overlay display, cancellation, and menu-bar residency. Claude validation on this machine covers adapter startup and the explicit authentication-required flow; authenticated transformation remains a follow-up on a machine with an eligible Claude account.

Official runtime references: [ACP Registry](https://agentclientprotocol.com/get-started/registry), [pnpm supply-chain security settings](https://pnpm.io/settings#minimumreleaseage), [Takumi Guard npm compatibility](https://shisho.dev/%64ocs/ja/t/guard/quickstart/npm/), [Node.js 24.19.0 distribution](https://nodejs.org/dist/v24.19.0/), and [Apple's Code Signing Requirement Language](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/RequirementLang/RequirementLang.html).
