<p align="center">
  <img src="apps/desktop/src-tauri/icons/icon-macos.svg" alt="Lens app icon" width="128" height="128">
</p>

<h1 align="center">Lens</h1>

Lens is a macOS menu bar app that helps you understand what you are reading. Select up to four application windows, and Claude or Codex turns their content into an **Interpretation**: a summary, an explanation, or another form suited to you.

## Why Lens?

Keep reading in the apps you already use, without repeatedly copying content into an AI chat. Use Lens to:

- Get the main points from a long page or document.
- Make unfamiliar concepts easier to understand.
- Bring information from several windows into one view.

Lens asks your Agent to use your existing instructions, memory, and preferences. As the selected content changes, it updates the Interpretation.

## Getting started

Lens is in early development. Packaged downloads are not yet available; check [GitHub Releases](https://github.com/japboy/lens/releases) for availability.

Settings are stored in `~/Library/Application Support/com.github.japboy.lens/settings.json`, alongside the managed Agent runtimes. Earlier builds used `~/Library/Application Support/Lens/settings.json`. Settings from that location are not migrated or loaded; upgrading starts with defaults unless settings already exist at the new location. The old files are left untouched.

You will need:

- An Apple silicon Mac running macOS 15.2 or later.
- Access to Claude or Codex and an internet connection.
- Accessibility permission to read window content. The macOS window picker authorizes image capture for the windows you select.

## How to use

1. Open Lens, then right-click its menu bar icon and choose **Settings...**.
2. Under **Agent → Connection**, choose **Claude** or **Codex**. Lens downloads the required Agent software on first selection; follow the authentication prompts and allow the requested macOS permissions.
3. Click the menu bar icon to select a window. Use **+** in the preview to add more windows, up to four, then click the checkmark to start.
4. Read the result in **Interpretation**. Open **Source** to inspect the content sent to the Agent.
5. Use **Pause Updates** and **Resume Updates** to control automatic updates. Close the Lens window to stop; click the menu bar icon again to select new windows.

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

For the three explanation presets, your Agent may present results as HTML or generated images in **Interpretation**, choosing the format according to the explanation, your preset instructions, and its actual capabilities without preferring either format by default. The bundled prompts explicitly consider available image-generation capabilities when choosing the format. If image generation is unavailable, it uses HTML. The bundled prompts instruct the Agent to design HTML for both light and dark modes, adapting colors while keeping text and diagrams readable. You can request an HTML presentation in an explanation preset. Evocative uses actual image generation and reports when that capability is unavailable instead of substituting HTML or diagrams. These previews support static content only; scripts and external assets do not load. Links open in your default browser.

Markdown responses and static HTML previews support TeX math: use `\(...\)` for inline equations and `\[...\]` or `$$...$$` for display equations. Markdown math is typeset when its output block settles; HTML math is typeset when the completed preview is prepared. Both use bundled KaTeX fonts with no network dependency. Single-dollar expressions remain ordinary text to avoid confusing prices with equations. Incomplete, unsupported, or oversized expressions retain their source text. In HTML, delimiters must stay within one text node; code samples, attributes, SVG, and existing MathML are left unchanged. Lens inserts static math and references bundled CSS and font files without enabling scripts. These display resources are not added to session history. Bundled explanation prompts request this syntax; previously saved prompt presets retain their existing instructions.

## Content access

Lens reads the windows you explicitly select and sends their text and captured images to your chosen Agent. Available content depends on what each application exposes to macOS Accessibility. If readable content is unavailable, Lens attempts to use a window image instead. The macOS window picker handles authorization for selected-window capture without requiring a separate global Screen Recording grant. Privacy & Security in Lens Settings also provides access to the system’s Screen & System Audio Recording settings. Lens does not record audio.
