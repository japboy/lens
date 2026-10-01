<p align="center">
  <img src="apps/desktop/src-tauri/icons/icon-macos.svg" alt="Lens app icon" width="128" height="128">
</p>

<h1 align="center">Lens</h1>

Lens is a macOS menu bar app that helps you understand what you are reading. Select up to four application windows, and an Agent such as ChatGPT Codex, Claude Code, or Google Antigravity turns their content into an **Interpretation**: a summary, an explanation, or another form suited to you.

## Why Lens?

Keep reading in the apps you already use, without repeatedly copying content into an AI chat. Use Lens to:

- Get the main points from a long page or document.
- Make unfamiliar concepts easier to understand.
- Bring information from several windows into one view.

Lens asks your Agent to use your existing instructions, memory, and preferences. As the selected content changes, it updates the Interpretation.

## Getting started

Lens is in early development. Download the latest packaged build from [GitHub Releases](https://github.com/japboy/lens/releases/latest). Follow the release notes to verify the download and install or update Lens.

Settings are stored in `~/Library/Application Support/com.github.japboy.lens/settings.json`, alongside the managed Agent runtimes.

You will need:

- An Apple silicon Mac running macOS 15.2 or later.
- Access to ChatGPT Codex, Claude Code, Google Antigravity, or another compatible Agent, and an internet connection.
- Accessibility permission to read window content. The macOS window picker authorizes image capture for the windows you select.

## How to use

1. Open Lens, then right-click its menu bar icon and choose **Settings...**.
2. Under **Agent → Connection**, choose **ChatGPT Codex**, **Claude Code**, or **Google Antigravity**. Lens downloads the required Agent software on first selection; follow the authentication prompts and allow the requested macOS permissions.
3. Click the menu bar icon to select a window. Use **+** in the preview to add more windows, up to four, then click the checkmark to start.
4. Read the result in **Interpretation**. Open **Source** to inspect the content sent to the Agent.
5. Use **Pause Updates** and **Resume Updates** to control automatic updates. Close the Lens window to stop; click the menu bar icon again to select new windows.

You can also connect any other Agent that supports the Agent Client Protocol (ACP) and HTTP MCP: install its CLI separately, then use **Add Preset** under **Settings → Agent → Connection** to configure its executable and arguments.

Automatic updates are spaced at least three minutes apart.

When viewing session history, **Interpretation** shows the most recent update with a successful HTML or image result recorded in the history. Later inputs, progress messages, or unsuccessful updates do not hide that result. A newer successful result replaces it, together with the explanation from that update. **Conversation** keeps the full history. For sessions without a successful HTML or image result, Interpretation shows the response to the last input.

To tailor the result, open **Settings → Agent → Prompt Presets**, choose a preset, and click **Use This Preset**. Lens initially uses **Conceptual** and includes four editable presets:

- **Conceptual** explains the central idea and how concepts relate.
- **Practical** starts with a worked example and actionable steps.
- **Analytical** explains relationships through quantities, equations, tables, or graphs when useful.
- **Evocative** translates the source into wordless generated imagery that evokes emotion and association. It requires an Agent with image-generation capability.

Conceptual, Practical, and Analytical describe explanation approaches. They combine words and useful visuals with consistent terminology, nearby explanations, and clear relationships while avoiding unnecessary decoration or repetition. Evocative prioritizes emotional resonance over explanatory precision, using one image or a meaningful sequence without labels or explanatory text. Its imagery may express any emotion suggested by the source, without a preference for positive feelings. Saved presets retain their names and instructions after app updates. Legacy preset IDs are migrated automatically while preserving the selected content; if an ID is already taken, the legacy preset is retained as a separate custom copy.

You can rename and edit any preset, then click **Save Preset**. Use **Duplicate** to create another preset or **Delete…** to remove one; the last preset cannot be deleted. **Reset All Presets…** replaces the entire collection, including your additions and edits, with the bundled presets. Saved changes and the active selection persist across restarts.

For quick switching, right-click the menu bar icon and choose **Prompt Presets**. Switching presets regenerates the Interpretation for the current sources. Set **Working Directory** in **Settings → Agent → Connection** if you want the Agent to use instructions and memory from a particular project.

For the three explanation presets, your Agent may present results as HTML or generated images in **Interpretation**, choosing the format according to the explanation, your preset instructions, and its actual capabilities without preferring either format by default. The bundled prompts explicitly consider available image-generation capabilities when choosing the format. If image generation is unavailable, it uses HTML. The bundled prompts instruct the Agent to design HTML for both light and dark modes, adapting colors while keeping text and diagrams readable. You can request an HTML presentation in an explanation preset. Evocative uses actual image generation and reports when that capability is unavailable instead of substituting HTML or diagrams. Rich and interactive HTML can use MCP Apps as described below. The bundled Lens HTML App replaces the former static HTML publishing tool. Previously saved HTML remains readable as source text in Conversation.

Markdown responses and static HTML previews support TeX math: use `\(...\)` for inline equations and `\[...\]` or `$$...$$` for display equations. Markdown math is typeset when its output block settles; HTML math is typeset when the completed preview is prepared. Both use bundled KaTeX fonts with no network dependency. Single-dollar expressions remain ordinary text to avoid confusing prices with equations. Incomplete, unsupported, or oversized expressions retain their source text. In HTML, delimiters must stay within one text node; code samples, attributes, SVG, and existing MathML are left unchanged. For saved static previews, Lens inserts static math and references bundled CSS and font files without enabling scripts. The built-in Lens HTML App receives a prepared display document with bundled math styles and fonts while retaining its authored JavaScript. External Apps own their math rendering. These display resources are not added to session history. Bundled explanation prompts request this syntax; previously saved prompt presets retain their existing instructions.

## Interactive Interpretations with MCP Apps

Lens hosts MCP Apps from servers you explicitly connect. Under **Settings → Agent → Connection → MCP**, the preset selector includes the protected **Lens HTML** built-in. Choose **Add Preset**, enter a unique **Name** and the server's **MCP Endpoint**, then click **Save Preset**. The selector chooses which preset to edit; all saved servers are available together. No sample server is registered by default. Example values appear only as placeholders. Saving closes the current Agent connection; the registry applies when you start the next Interpretation. This registry currently supports Streamable HTTP endpoints without authentication. Stdio servers, authenticated endpoints, and MCP tools privately configured in an Agent are not automatically imported into Lens.

The Agent can use an appropriate connected App tool to produce an Interpretation. Lens presents its App resource with the original tool input and result, and App tool calls are restricted to that originating server's App-visible tools. If an appropriate external HTML renderer is unavailable, Lens provides **`lens_rich_html.render_html`** to render a complete self-contained HTML/CSS/JavaScript document in the same Host. Generated content runs in an isolated document with no Lens native IPC authority. Supported external Apps can load resources or connect to network origins declared by their App CSP; dedicated origins, device permissions, external frames, and other unsupported capabilities report an error. The bundled HTML fallback uses self-contained assets.

An App can supply updated context or propose a message. A proposed message appears as a Lens draft; it does not automatically start an Agent turn. Click **Send to Agent** to submit the message with that App's latest context to the same live Agent session, or **Discard** to dismiss it. Follow-ups are unavailable while the Agent is busy, source monitoring is paused, or the original session has closed. Use **Close App** and **Reopen App** to control the display. Reopening creates a new display lease and clears its previous draft and interaction context.

Apps retained in the current Lens operation's response history can be reopened; when their original Agent connection has ended, they are read-only. App resources are temporary and are discarded when that Lens operation is replaced or the application exits. Restarting Lens does not restore interactive App artifacts from provider session history. Successful built-in HTML publications can be read from saved provider history as inert HTML source text after validating their origin, tool call, receipt and content hash. The former `lens_output.publish_html` tool and its dedicated server are removed; older saved HTML remains readable. Links proposed by Apps appear in Lens with **Open in Browser** and **Discard** controls. Opening a link uses the default browser without starting an Agent turn.

## Content access

Lens reads the windows you explicitly select and sends their text and captured images to your chosen Agent. Available content depends on what each application exposes to macOS Accessibility. If readable content is unavailable, Lens attempts to use a window image instead. The macOS window picker handles authorization for selected-window capture without requiring a separate global Screen Recording grant. Privacy & Security in Lens Settings also provides access to the system’s Screen & System Audio Recording settings. Lens does not record audio.
