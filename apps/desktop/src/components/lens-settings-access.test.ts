// @vitest-environment jsdom
import { expect, it } from "vitest";
import { LensSettingsView } from "./lens-settings-view";
import type { AccessibilityPermissionState } from "../application/accessibility-permission-controller";

it.each([
  [{ stage: "inactive" }, "Checking…", false],
  [{ stage: "checking" }, "Checking…", false],
  [{ stage: "allowed" }, "Ready", false],
  [{ stage: "required" }, "Permission required", true],
  [{ stage: "restricted" }, "Access restricted", false],
  [{ stage: "unsupported" }, "Not supported", false],
  [{ stage: "failed", message: "Unavailable" }, "Access check failed", false],
] satisfies [AccessibilityPermissionState, string, boolean][])(
  "renders access %j with only the declared action",
  async (permission, label, hasAction) => {
    const view = new LensSettingsView();
    view.permission = permission;
    document.body.append(view);
    try {
      await view.updateComplete;
      expect(view.shadowRoot?.querySelector(".permission-row output")?.textContent).toContain(
        label,
      );
      expect(Boolean(view.shadowRoot?.querySelector(".permission-row button"))).toBe(hasAction);
    } finally {
      view.remove();
    }
  },
);
