// @vitest-environment jsdom
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import type { LensSettingsView } from "./lens-settings-view";
import { SETTINGS_INTENT_EVENT } from "../../contracts/events";
import {
  SOURCE_CAPABILITIES,
  type AccessStatus,
  type CapabilityInspection,
} from "../../contracts/resource-state";

beforeAll(async () => {
  await import("./lens-settings-view");
});
afterEach(() => {
  document.body.replaceChildren();
});
const reports = (status: AccessStatus): CapabilityInspection[] =>
  SOURCE_CAPABILITIES.map((capability) => ({
    capability,
    status,
    request_action:
      status === "permission_required" && capability === "structured_extraction"
        ? "request_permission"
        : null,
    diagnostics: [`${capability} detail`],
  }));
async function mount(status: AccessStatus) {
  const view = document.createElement("lens-settings-view") as LensSettingsView;
  view.active = true;
  view.permission = { stage: "ready", capabilities: reports(status) };
  document.body.append(view);
  await view.updateComplete;
  return view;
}

it("displays independent capability statuses and only the advertised explicit native request", async () => {
  const view = await mount("permission_required");
  const section = view.shadowRoot!.querySelector('section[aria-labelledby="permission-heading"]')!;
  expect(section.querySelectorAll("h3")).toHaveLength(4);
  expect(section.textContent).toContain("Permission required");
  expect(section.textContent).toContain("observation detail");
  const buttons = section.querySelectorAll<HTMLButtonElement>("button");
  expect(buttons).toHaveLength(1);
  expect(buttons[0]!.textContent).toContain("Request access");
  const intent = vi.fn<(event: Event) => void>();
  view.addEventListener(SETTINGS_INTENT_EVENT, intent);
  buttons[0]!.click();
  expect(intent).toHaveBeenCalledTimes(1);
  expect((intent.mock.calls[0]![0] as CustomEvent).detail).toEqual({
    type: "request-source-access",
    capability: "structured_extraction",
  });
});

it("does not offer false grant controls for restricted, unsupported or failed inspections", async () => {
  for (const status of ["restricted", "unsupported", "failed", "available"] as const) {
    const view = await mount(status);
    const section = view.shadowRoot!.querySelector(
      'section[aria-labelledby="permission-heading"]',
    )!;
    expect(section.querySelector("button")).toBeNull();
    expect(section.querySelectorAll("output")).toHaveLength(4);
    view.remove();
  }
});

it("keeps macOS settings navigation available without a snapshot and hides it for other explicit platforms", async () => {
  const view = await mount("available");
  view.dataset.platform = "macos";
  view.requestUpdate();
  await view.updateComplete;
  expect(
    view.shadowRoot!.querySelector('[aria-labelledby="screen-recording-heading"] button'),
  ).not.toBeNull();
  view.dataset.platform = "windows";
  view.requestUpdate();
  await view.updateComplete;
  expect(
    view.shadowRoot!.querySelector<HTMLElement>('[aria-labelledby="screen-recording-heading"]')!
      .hidden,
  ).toBe(true);
  expect(
    view.shadowRoot!.querySelector<HTMLButtonElement>(
      '[aria-labelledby="screen-recording-heading"] button',
    )!.disabled,
  ).toBe(true);
});
