// @vitest-environment jsdom
import { beforeAll, afterEach, expect, it } from "vitest";
import type { LitElement } from "lit";
let preview: typeof import("../src/main");
beforeAll(async () => {
  window.matchMedia = () =>
    ({
      matches: false,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
  HTMLElement.prototype.scrollTo = () => undefined;
  document.body.innerHTML =
    '<select id="view"><option>about</option><option>settings</option><option>target-selection</option><option>overlay</option><option>settings-recovery</option></select><select id="scenario"><option>ready</option><option>loading</option><option>failed</option></select><select id="platform"><option>macos</option></select><div id="mount"></div><pre id="intents"></pre>';
  preview = await import("../src/main");
});
afterEach(() => {
  preview.intents.length = 0;
});
async function mount(view: string, scenario = "ready") {
  document.querySelector<HTMLSelectElement>("#view")!.value = view;
  document.querySelector<HTMLSelectElement>("#scenario")!.value = scenario;
  const element = preview.show();
  await element.updateComplete;
  for (const child of element.shadowRoot?.querySelectorAll("*") ?? [])
    if ("updateComplete" in child) await (child as LitElement).updateComplete;
  return element;
}
it.each(["about", "settings", "target-selection", "overlay", "settings-recovery"])(
  "renders actual %s ready/loading contracts",
  async (name) => {
    for (const state of ["ready", "loading"]) {
      const element = await mount(name, state);
      expect(element.localName).toBe(`lens-${name}-view`);
      expect(element.shadowRoot!.textContent!.trim().length).toBeGreaterThan(100);
    }
  },
);
it.each(["about", "settings", "target-selection", "overlay", "settings-recovery"])(
  "announces the %s failed resource",
  async (name) => {
    const element = await mount(name, "failed");
    const alerts = [...element.shadowRoot!.querySelectorAll('[role="alert"]')]
      .map((alert) => alert.textContent)
      .join(" ");
    expect(alerts).toContain("Fixture");
  },
);
it("shows actual target card and emits confirm/add intents without native global", async () => {
  const view = await mount("target-selection");
  expect(view.shadowRoot!.textContent).toContain("Independent UI preview");
  view.shadowRoot!.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
  view.shadowRoot!.querySelector<HTMLButtonElement>('[aria-label="Add Another Window"]')!.click();
  expect(preview.intents).toEqual([
    { event: "lens-target-selection-intent", detail: { type: "confirm" } },
    { event: "lens-target-selection-intent", detail: { type: "add" } },
  ]);
  expect("__TAURI_INTERNALS__" in window).toBe(false);
});
it("renders production markdown narrative inside actual Overlay", async () => {
  const view = await mount("overlay");
  await new Promise((resolve) => setTimeout(resolve, 60));
  expect(view.shadowRoot!.textContent).toContain("Actual Lens rendering");
  expect(view.shadowRoot!.querySelector("lens-markdown")).not.toBeNull();
});
it("About retry maps semantic intent; Recovery exposes retry and confirmation independently", async () => {
  const about = await mount("about");
  if (!("releaseAvailability" in about)) throw new Error("Expected About view");
  about.releaseAvailability = { revision: 2, stage: "failed" };
  await about.updateComplete;
  const retry = about.shadowRoot!.querySelector<HTMLButtonElement>("button")!;
  expect(retry).not.toBeNull();
  retry.click();
  expect(preview.intents).toContainEqual({
    event: "lens-about-intent",
    detail: { type: "retry-update-check" },
  });
  const recovery = await mount("settings-recovery");
  [...recovery.shadowRoot!.querySelectorAll<HTMLButtonElement>("button")]
    .find((button) => button.textContent?.includes("Retry"))!
    .click();
  expect(preview.intents).toContainEqual({ event: "settings-recovery-action", detail: "retry" });
});
it("Settings draft survives equivalent parent model publication", async () => {
  const settings = await mount("settings");
  const prompt = settings.shadowRoot!.querySelector<LitElement & { agentPromptTemplate?: unknown }>(
    "lens-prompt-settings",
  )!;
  expect(prompt).not.toBeNull();
  const textarea = prompt.querySelector<HTMLTextAreaElement>("textarea");
  expect(textarea).not.toBeNull();
  textarea!.value = "Local unsaved fixture";
  textarea!.dispatchEvent(new Event("input", { bubbles: true }));
  await prompt.updateComplete;
  if ("model" in settings) {
    settings.model = { ...settings.model! };
    await settings.updateComplete;
    await prompt.updateComplete;
  }
  expect(prompt.querySelector<HTMLTextAreaElement>("textarea")!.value).toBe(
    "Local unsaved fixture",
  );
});
