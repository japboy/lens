# PersonalLens

PersonalLens is a menu-bar-first application that reads the Accessibility Tree exposed by another macOS application and transforms that information through an ACP agent selected by the user.

The PoC focuses on accessibility-based text extraction and ACP-based transformation. It does not handle images, OCR, audio, or real-time content translation. AXObserver-based selected-window geometry following is included.

## Requirements

- macOS 15.2 or later
- Xcode Command Line Tools
- Rust 1.88 or later
- Node.js 24 or later
- Accessibility permission

macOS 15.2 is the minimum because it is the first version that provides the public `includedWindows` API needed to deterministically obtain the selected `SCWindow` from the `SCContentFilter` returned by the native picker.

## Development

```sh
npm install
npm run verify
npm run tauri -- dev
```

The application bundle does not contain Claude, Codex, their ACP adapters, or a separate Node.js runtime. The first explicit selection of an Agent installs only that Agent's approved runtime under the application-local data directory. PersonalLens validates the official ACP Registry package identity, installs from an embedded exact npm lock, verifies the pinned Node.js `24.19.0` archive SHA-256 digest, and verifies the Developer ID team and signing identifier of Node and the provider executable before launch. Runtime resolution never uses `PATH`, globally installed npm packages, or `npx @latest`.

To produce a debug application bundle:

```sh
npm run tauri -- build --debug
```

The application creates no ordinary WebView window at startup. Left-clicking the `Lens` menu-bar item immediately opens the native Lens Target picker when an authenticated Agent is selected. When target selection is unavailable, the template icon is rendered at half opacity and the click has no effect. Right-clicking the item opens this native menu. When Accessibility permission is missing at first launch, the application invokes the standard macOS permission onboarding flow.

- `Select Lens Target…`
- `AI Agent` (Claude / Codex)
- `Working Directory: <absolute path>…`
- `Settings…`
- `Quit PersonalLens`

An Agent receives a check mark only after PersonalLens installs and verifies its managed runtime and then verifies its existing authentication. Codex is verified by ACP session creation; Claude is first verified by the adapter's official CLI authentication-status command because Claude Agent ACP does not reject unauthenticated session creation. Clicking an unauthenticated Agent opens the adapter-owned authentication flow in Settings. `Select Lens Target…` remains disabled until an authenticated Agent is selected. Clicking the Working Directory item opens the native macOS folder picker and updates the path shown in the menu.

Settings provides Agent selection, adapter-owned authentication, reauthentication, sign-out, Working Directory, and Accessibility permission controls. Reauthentication and sign-out first show a native confirmation dialog. Settings contains no Lens Target button; Lens Target selection is a menu-bar action. Variable authentication content remains contained in a scrollable, responsive Settings layout.

The Lens overlay opens centered at 80% of the selected window's width and height, follows its move/resize lifecycle, and uses a lightly translucent native background material. Translation is the default tab. ACP text deltas render through an append-only Markdown DOM with a stream cursor and reader-aware auto-scroll; terminal output settles once as sanitized GitHub Flavored Markdown. Source keeps the extracted Accessibility text and diagnostics available without flashing them as the Agent result. Loading uses only bundled assets.

## Security Boundary

- PersonalLens stores only the last successfully selected agent and working directory. Authentication status is verified at runtime and is not persisted.
- Managed Agent runtimes are provider-specific, versioned application data. Interrupted installs remain in staging and are never selected; an invalid existing install is quarantined before replacement.
- It does not read or write OAuth tokens, API keys, or Claude/Codex credential files.
- Accessibility extraction is limited to a snapshot of the target explicitly selected by the user.
- PersonalLens exposes no coding client capabilities to the ACP agent.
- Authentication methods and logout come from the agent adapter. Codex browser authentication and Claude terminal authentication start adapter-owned flows; ACP `logout` delegates sign-out to the selected adapter. PersonalLens stores no tokens, API keys, or credential files.

## Validation

`npm run verify` runs publication-boundary and repository-language policies, Lit frontend type checking/build/Vitest, and Rust check/format/Clippy/unit tests.

An on-device managed-runtime install and verification can be run independently for each Agent:

```sh
PERSONAL_LENS_VALIDATE_RUNTIME=claude npm run tauri -- dev
PERSONAL_LENS_VALIDATE_RUNTIME=codex npm run tauri -- dev
```

On-device PoC validation covers native Safari window selection, Accessibility extraction beyond the visible viewport, LensInput generation, managed Codex ACP transformation, Lit overlay display, cancellation, and menu-bar residency. Claude validation on this machine covers adapter startup and the explicit authentication-required flow; authenticated transformation remains a follow-up on a machine with an eligible Claude account.

Official runtime references: [ACP Registry](https://agentclientprotocol.com/get-started/registry), [npm `ci`](https://docs.npmjs.com/cli/commands/npm-ci/), [Node.js 24.19.0 distribution](https://nodejs.org/dist/v24.19.0/), and [Apple's Code Signing Requirement Language](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/RequirementLang/RequirementLang.html).
