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

You will need:

- An Apple silicon Mac running macOS 15.2 or later.
- Access to Claude or Codex and an internet connection.
- Accessibility permission to read window content, and Screen Recording permission to capture images.

## How to use

1. Open Lens, then right-click its menu bar icon and choose **Settings...**.
2. Under **Agent → Connection**, choose **Claude** or **Codex**. Lens downloads the required Agent software on first selection; follow the authentication prompts and allow the requested macOS permissions.
3. Click the menu bar icon to select a window. Use **+** in the preview to add more windows, up to four, then click the checkmark to start.
4. Read the result in **Interpretation**. Open **Source** to inspect the content sent to the Agent.
5. Use **Pause Updates** and **Resume Updates** to control automatic updates. Close the Lens window to stop; click the menu bar icon again to select new windows.

Automatic updates are spaced at least three minutes apart.

To tailor the result, open **Settings → Agent → Prompt Presets**, choose a preset, and click **Use This Preset**. Lens initially uses **Visual Learner** and includes four editable presets:

- **Visual Learner** leads with an infographic and follows with supporting explanation.
- **Conceptual Learner** explains the central idea and how concepts relate.
- **Practical Learner** starts with a worked example and actionable steps.
- **Analytical Learner** explains relationships through quantities, equations, tables, or graphs when useful.

You can rename and edit any preset, then click **Save Preset**. Use **Duplicate** to create another preset or **Delete…** to remove one; the last preset cannot be deleted. **Reset All Presets…** replaces the entire collection, including your additions and edits, with the four initial presets. Saved changes and the active selection persist across restarts.

For quick switching, right-click the menu bar icon and choose **Prompt Presets**. Switching presets regenerates the Interpretation for the current sources. Set **Working Directory** in **Settings → Agent → Connection** if you want the Agent to use instructions and memory from a particular project.

Your Agent may present results as HTML or generated images in **Interpretation**, depending on its available capabilities and your preset instructions. You can request an HTML presentation in any preset. These previews support static content only; scripts and external assets do not load. Links open in your default browser.

## Content access

Lens reads the windows you explicitly select and sends their text and captured images to your chosen Agent. Available content depends on what each application exposes to macOS Accessibility. If readable content is unavailable, Lens attempts to use a window image instead. Text remains usable without Screen Recording permission when Accessibility content is available.
