import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const componentStyles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");
const documentStyles = readFileSync(new URL("./styles/document.css", import.meta.url), "utf8");

describe("canonical application icon presentation", () => {
  it("preserves the generated composition without another mask or shadow", () => {
    const declarations = componentStyles.match(/\.overlay-app-icon \{([^}]+)\}/u)?.[1];
    expect(declarations).toContain("object-fit: contain;");
    expect(declarations).not.toMatch(/border-radius|box-shadow/u);
  });
});

describe("macOS Settings surface colors", () => {
  it("derives low-contrast groups from AppKit's dynamic window and content colors", () => {
    const macosSettingsColors = componentStyles.match(
      /html\[data-view="settings"\]\[data-platform="macos"\] \{(?<declarations>.*?)\n\}/s,
    )?.groups?.declarations;
    const macosSelectionColors = componentStyles.match(
      /@supports \(color: -apple-system-selected-content-background\) \{[\s\S]*?html\[data-view="settings"\]\[data-platform="macos"\] \{(?<declarations>.*?)\n  \}/s,
    )?.groups?.declarations;

    expect(macosSettingsColors).toBeDefined();
    expect(macosSettingsColors).toContain("--settings-backdrop: Window;");
    expect(macosSettingsColors).toContain(
      "--settings-group-background: color-mix(in srgb, Window 80%, Canvas 20%);",
    );
    expect(macosSettingsColors).not.toMatch(/--settings-group-background:\s*Canvas;/);
    expect(macosSettingsColors).not.toContain("-apple-system-grouped-background");
    expect(macosSettingsColors).not.toContain("-apple-system-secondary-grouped-background");
    expect(macosSelectionColors).toContain(
      "--settings-selection-background: -apple-system-selected-content-background;",
    );
    expect(macosSelectionColors).toContain(
      "--settings-selection-unemphasized-background: -apple-system-unemphasized-selected-content-background;",
    );
    expect(macosSelectionColors).toContain("--settings-focus-ring: -webkit-focus-ring-color;");
  });

  it("uses one persistent sidebar throughout the native window width range", () => {
    const shell = componentStyles.match(/\.settings-shell \{(?<declarations>.*?)\n\}/s)?.groups
      ?.declarations;
    const sidebar = componentStyles.match(/\.settings-sidebar \{(?<declarations>.*?)\n\}/s)?.groups
      ?.declarations;
    const sidebarNavigation = componentStyles.match(
      /\.settings-sidebar nav \{(?<declarations>.*?)\n\}/s,
    )?.groups?.declarations;
    const selectedNavigation = componentStyles.match(
      /\.settings-nav-item\[aria-current="page"\] \{(?<declarations>.*?)\n\}/s,
    )?.groups?.declarations;

    expect(shell).toContain("grid-template-columns: var(--settings-sidebar-width) minmax(0, 1fr);");
    expect(shell).toContain("--settings-sidebar-width: 188px;");
    expect(shell).toContain("--settings-detail-inline-padding: 26px;");
    expect(shell).toContain("--settings-content-max-width: 680px;");
    expect(shell).toContain("grid-template-rows: minmax(0, 1fr);");
    expect(sidebar).toContain("grid-template-rows: minmax(0, 1fr) auto;");
    expect(sidebar).toContain("overflow: hidden;");
    expect(sidebarNavigation).toContain("overflow-y: auto;");
    expect(componentStyles).toContain(".settings-sidebar-status {");
    expect(componentStyles).not.toContain(".settings-footer");
    expect(selectedNavigation).toContain("background: var(--settings-selection-background);");
    expect(selectedNavigation).toContain("color: var(--settings-selection-foreground);");
    expect(documentStyles).toContain("--settings-sidebar-background:");
    expect(documentStyles).toContain(
      "--settings-selection-background: -apple-system-selected-content-background;",
    );
    expect(componentStyles).toContain(
      '.settings-shell[data-window-emphasis="unemphasized"] .settings-nav-item[aria-current="page"]',
    );
    expect(componentStyles).toContain(
      '.settings-nav-item:hover:not(:disabled):not([aria-current="page"])',
    );
    expect(componentStyles).not.toContain(".settings-compact-navigation");
    expect(componentStyles).not.toMatch(/\.settings-sidebar \{[^}]*display: none;/s);
  });

  it("keeps the rendered prompt visible in a bounded static section", () => {
    const header = componentStyles.match(/\.prompt-preview-header \{(?<declarations>.*?)\n\}/s)
      ?.groups?.declarations;
    const output = componentStyles.match(/\.prompt-preview-output \{(?<declarations>.*?)\n\}/s)
      ?.groups?.declarations;

    expect(header).toContain("display: flex;");
    expect(header).toContain("padding: 11px 14px;");
    expect(header).toContain("border-bottom: 1px solid var(--settings-group-border);");
    expect(output).toContain("min-height: 180px;");
    expect(output).toContain("max-height: 420px;");
    expect(output).toContain("overflow: auto;");
    expect(componentStyles).not.toContain("prompt-preview-disclosure");
    expect(componentStyles).not.toContain("prompt-preview-disclosure-icon");
  });
});

describe("Lens overlay presentation", () => {
  it("uses equal header edge spacing around the app icon and close control", () => {
    const overlayHeader = componentStyles.match(/\.overlay-header \{(?<declarations>.*?)\n\}/s)
      ?.groups?.declarations;
    const closeButton = componentStyles.match(/\.close-button \{(?<declarations>.*?)\n\}/s)?.groups
      ?.declarations;

    expect(overlayHeader).toContain("padding: 7px 14px;");
    expect(overlayHeader).not.toMatch(/padding:\s*7px\s+\d+px\s+7px\s+\d+px;/);
    expect(closeButton).toContain("width: 24px;");
    expect(closeButton).toContain("height: 24px;");
  });

  it("reserves snackbar clearance in every scrollable panel", () => {
    const progressClearance = componentStyles.match(
      /\.overlay-main\[data-progress="true"\] \.lens-content,\s*\.overlay-main\[data-progress="true"\] \.extraction-diagnostics \{(?<declarations>.*?)\n\}/s,
    )?.groups?.declarations;

    expect(progressClearance).toContain("padding-block-end: 88px;");
  });

  it("keeps the WebView surface transparent with an opaque accessibility fallback", () => {
    const overlayShell = componentStyles.match(/\.overlay-shell \{(?<declarations>.*?)\n\}/s)
      ?.groups?.declarations;
    const sourceSummary = componentStyles.match(
      /\.overlay-source-summary \{(?<declarations>.*?)\n\}/s,
    )?.groups?.declarations;
    const progressSnackbar = componentStyles.match(
      /\.lens-progress-snackbar \{(?<declarations>.*?)\n\}/s,
    )?.groups?.declarations;
    const progressRegion = componentStyles.match(
      /\.lens-progress-region \{(?<declarations>.*?)\n\}/s,
    )?.groups?.declarations;
    const overlayFooter = componentStyles.match(/\.overlay-footer \{(?<declarations>.*?)\n\}/s)
      ?.groups?.declarations;

    expect(overlayShell).toContain("background: transparent;");
    expect(sourceSummary).toContain("background: color-mix(in srgb, AccentColor 8%, transparent);");
    expect(progressSnackbar).toContain("background: color-mix(in srgb, Canvas 82%, transparent);");
    expect(progressSnackbar).toContain("backdrop-filter: blur(18px) saturate(150%);");
    expect(progressRegion).toContain("bottom: calc(100% + 14px);");
    expect(overlayFooter).toContain("position: relative;");
    expect(componentStyles).toMatch(
      /@media \(prefers-reduced-transparency: reduce\), \(prefers-contrast: more\) \{\s*\.overlay-shell \{[^}]*background: Canvas;/s,
    );
    expect(componentStyles).toMatch(
      /@media \(prefers-reduced-transparency: reduce\), \(prefers-contrast: more\) \{[\s\S]*\.lens-progress-snackbar \{[^}]*background: Canvas;[^}]*backdrop-filter: none;/,
    );
  });
});
