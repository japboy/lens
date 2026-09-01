import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const componentStyles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

describe("macOS Settings surface colors", () => {
  it("derives low-contrast groups from AppKit's dynamic window and content colors", () => {
    const macosSettingsColors = componentStyles.match(
      /html\[data-view="settings"\]\[data-platform="macos"\] \{(?<declarations>.*?)\n\}/s,
    )?.groups?.declarations;

    expect(macosSettingsColors).toBeDefined();
    expect(macosSettingsColors).toContain("--settings-backdrop: Window;");
    expect(macosSettingsColors).toContain(
      "--settings-group-background: color-mix(in srgb, Window 80%, Canvas 20%);",
    );
    expect(macosSettingsColors).not.toMatch(/--settings-group-background:\s*Canvas;/);
    expect(macosSettingsColors).not.toContain("-apple-system-grouped-background");
    expect(macosSettingsColors).not.toContain("-apple-system-secondary-grouped-background");
  });
});

describe("Lens overlay presentation", () => {
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
