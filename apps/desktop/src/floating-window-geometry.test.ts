// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { installFloatingWindowGeometry } from "./floating-window-geometry";

describe("constructor-owned floating window geometry", () => {
  it.each(["overlay", "target-selection"] as const)(
    "applies %s geometry independently of palette state",
    (view) => {
      const root = document.createElement("html");
      installFloatingWindowGeometry({ view, platform: "macos" }, root, { corner_radius: 12 });
      expect(root.style.getPropertyValue("--floating-window-corner-radius")).toBe("12px");
      expect(root.dataset.nativeControls).toBeUndefined();
    },
  );

  it.each([
    null,
    undefined,
    {},
    { corner_radius: NaN },
    { corner_radius: Infinity },
    { corner_radius: -1 },
    { corner_radius: 65 },
    { corner_radius: "12px" },
  ])("clears stale geometry for invalid constructor state %j", (value) => {
    const root = document.createElement("html");
    root.style.setProperty("--floating-window-corner-radius", "12px");
    installFloatingWindowGeometry({ view: "overlay", platform: "macos" }, root, value);
    expect(root.style.getPropertyValue("--floating-window-corner-radius")).toBe("");
  });

  it.each(["settings", "settings-recovery", "about"] as const)(
    "does not round %s using floating geometry",
    (view) => {
      const root = document.createElement("html");
      installFloatingWindowGeometry({ view, platform: "macos" }, root, { corner_radius: 12 });
      expect(root.style.getPropertyValue("--floating-window-corner-radius")).toBe("");
    },
  );
});
