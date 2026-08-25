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
npm run sidecars:build
npm run verify
npm run tauri -- dev
```

`sidecars:build` downloads pinned official npm packages and the Node.js `24.19.0` arm64 runtime, then verifies the SHA-256 digest of the Node archive. The same build runs before `tauri dev/build`, and its outputs are bundled under `.app/Contents/Resources/sidecars`. Runtime resolution never uses `PATH`, globally installed npm packages, or `npx @latest`.

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

An Agent receives a check mark only after PersonalLens verifies its existing authentication. Codex is verified by ACP session creation; Claude is first verified by the bundled adapter's official CLI authentication-status command because Claude Agent ACP does not reject unauthenticated session creation. Clicking an unauthenticated Agent opens the adapter-owned authentication flow in Settings. `Select Lens Target…` remains disabled until an authenticated Agent is selected. Clicking the Working Directory item opens the native macOS folder picker and updates the path shown in the menu.

Settings provides Agent selection, adapter-owned authentication, reauthentication, sign-out, Working Directory, and Accessibility permission controls. Reauthentication and sign-out first show a native confirmation dialog. Settings contains no Lens Target button; Lens Target selection is a menu-bar action. Variable authentication content remains contained in a scrollable, responsive Settings layout.

The Lens overlay opens centered at 80% of the selected window's width and height, follows its move/resize lifecycle, and uses a lightly translucent native background material. Translation is the default tab. ACP text deltas render through an append-only Markdown DOM with a stream cursor and reader-aware auto-scroll; terminal output settles once as sanitized GitHub Flavored Markdown. Source keeps the extracted Accessibility text and diagnostics available without flashing them as the Agent result. Loading uses only bundled assets.

## Security Boundary

- PersonalLens stores only the last successfully selected agent and working directory. Authentication status is verified at runtime and is not persisted.
- It does not read or write OAuth tokens, API keys, or Claude/Codex credential files.
- Accessibility extraction is limited to a snapshot of the target explicitly selected by the user.
- PersonalLens exposes no coding client capabilities to the ACP agent.
- Authentication methods and logout come from the agent adapter. Codex browser authentication and Claude terminal authentication start adapter-owned flows; ACP `logout` delegates sign-out to the selected adapter. PersonalLens stores no tokens, API keys, or credential files.

## Validation

`npm run verify` runs publication-boundary and repository-language policies, Lit frontend type checking/build/Vitest, the sidecar lock and version smoke test, and Rust check/format/Clippy/unit tests.

On-device PoC validation covers native Safari window selection, Accessibility extraction beyond the visible viewport, LensInput generation, bundled Codex ACP transformation, Lit overlay display, cancellation, and menu-bar residency. Claude validation on this machine covers adapter startup and the explicit authentication-required flow; authenticated transformation remains a follow-up on a machine with an eligible Claude account.
