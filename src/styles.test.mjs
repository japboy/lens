import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const componentStyles = readFileSync(new URL("./styles.css", import.meta.url), "utf8");

describe("macOS Settings surface colors", () => {
  it("uses the AppKit window background semantic behind content groups", () => {
    expect(componentStyles).toContain("--settings-backdrop: Window;");
    expect(componentStyles).toContain("--settings-group-background: Canvas;");
    expect(componentStyles).not.toContain("-apple-system-grouped-background");
    expect(componentStyles).not.toContain("-apple-system-secondary-grouped-background");
  });
});
