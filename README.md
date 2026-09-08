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
2. Choose **Claude** or **Codex**. Lens downloads the required Agent software on first selection; follow the authentication prompts and allow the requested macOS permissions.
3. Click the menu bar icon to select a window. Use **+** in the preview to add more windows, up to four, then click the checkmark to start.
4. Read the result in **Interpretation**. Open **Source** to inspect the content sent to the Agent.
5. Use **Pause Updates** and **Resume Updates** to control automatic updates. Close the Lens window to stop; click the menu bar icon again to select new windows.

Automatic updates are spaced at least three minutes apart.

To tailor the result, edit **Settings → Agent Prompt**. Set **Working Directory** if you want the Agent to use instructions and memory from a particular project.

Your Agent may present results as HTML alongside images in **Interpretation**, with no additional setup. You can request an HTML presentation in **Settings → Agent Prompt**. These previews support static content only; scripts and external assets do not load. Links open in your default browser.

## Content access

Lens reads the windows you explicitly select and sends their text and captured images to your chosen Agent. Available content depends on what each application exposes to macOS Accessibility. If readable content is unavailable, Lens attempts to use a window image instead. Text remains usable without Screen Recording permission when Accessibility content is available.
