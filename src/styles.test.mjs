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
