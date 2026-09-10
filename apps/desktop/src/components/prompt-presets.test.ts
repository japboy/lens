// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { LensPromptSettings } from "./lens-prompt-settings";
import type { AgentPromptTemplate, PromptPresetCollection } from "../types";
import type { PromptIntent } from "./events";

const template: AgentPromptTemplate = {
  schema_version: 1,
  common: "Explain. {turn_instruction}",
  full_projection: "Use initial content.",
  source_checkpoint: "Update {base_revision} to {target_revision}.",
  current_projection_retry: "Retry {applied_revision}.",
};
function collection(): PromptPresetCollection {
  return {
    schema_version: 2,
    revision: 1,
    execution_revision: 1,
    selected_id: "visual-learner",
    presets: [
      {
        id: "visual-learner",
        name: "Visual Learner",

        revision: 1,
        template: { ...template },
      },
      {
        id: "practical-learner",
        name: "Practical Learner",

        revision: 2,
        template: { ...template, common: "Examples. {turn_instruction}" },
      },
    ],
  };
}
async function mount() {
  const element = new LensPromptSettings();
  element.agentPromptTemplate = template;
  element.promptPresets = collection();
  const intents: PromptIntent[] = [];
  element.addEventListener("lens-prompt-intent", (event) =>
    intents.push((event as CustomEvent<PromptIntent>).detail),
  );
  document.body.append(element);
  await element.updateComplete;
  return { element, intents };
}
function button(element: HTMLElement, label: string): HTMLButtonElement {
  return [...element.querySelectorAll("button")].find(
    (item) => item.textContent?.trim() === label,
  )!;
}
async function edit(element: LensPromptSettings, selector: string, value: string) {
  const input = element.querySelector<HTMLInputElement>(selector)!;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await element.updateComplete;
}
async function browse(element: LensPromptSettings, id: string) {
  const select = element.querySelector<HTMLSelectElement>("#prompt-preset-list")!;
  select.value = id;
  select.dispatchEvent(new Event("change", { bubbles: true }));
  await element.updateComplete;
}
afterEach(() => document.body.replaceChildren());
describe("prompt preset editing", () => {
  it("groups the editor with sibling advanced and preview disclosures and separate reset", async () => {
    const { element } = await mount();
    const group = element.querySelector(".prompt-presets")!;
    const advanced = element.querySelector<HTMLDetailsElement>(".prompt-advanced")!;
    const preview = element.querySelector<HTMLDetailsElement>(".prompt-preview")!;
    expect(advanced.parentElement).toBe(group);
    expect(preview.parentElement).toBe(group);
    expect(advanced.open).toBe(false);
    expect(preview.open).toBe(false);
    expect(group.contains(button(element, "Use This Preset"))).toBe(false);
    expect(group.contains(button(element, "Reset All Presets…"))).toBe(false);
    expect(group.contains(button(element, "Save Preset"))).toBe(true);
    expect(element.querySelector(".prompt-composition")).toBeNull();
    expect(element.querySelector("#prompt-preset-replacement")).toBeNull();
    advanced.open = true;
    const request = element.querySelector<HTMLSelectElement>("#prompt-request-mode")!;
    request.value = "source_checkpoint";
    request.dispatchEvent(new Event("change", { bubbles: true }));
    await element.updateComplete;
    expect(element.querySelector<HTMLTextAreaElement>("#prompt-request-editor")!.value).toBe(
      template.source_checkpoint,
    );
    expect(advanced.open).toBe(true);
    expect(preview.open).toBe(false);
  });
  it("saves bundled preset metadata and prompt together without creating a Custom preset", async () => {
    const { element, intents } = await mount();
    await edit(element, "#prompt-preset-name", "My Visual Style");
    await edit(element, "#prompt-editor", "My instruction. {turn_instruction}");
    button(element, "Save Preset").click();
    expect(intents.at(-1)).toMatchObject({
      change: {
        type: "update",
        id: "visual-learner",
        name: "My Visual Style",
        template: { common: "My instruction. {turn_instruction}" },
      },
    });
    expect(button(element, "Revert Changes").hidden).toBe(true);
    expect(
      element.querySelector("#prompt-preset-name")?.classList.contains("prompt-preset-field"),
    ).toBe(true);
  });
  it("clears every draft only after a confirmed successful full reset", async () => {
    const { element } = await mount();
    const next = collection();
    next.presets.push({ id: "mine", name: "Mine", revision: 1, template });
    element.promptPresets = next;
    await element.updateComplete;
    await edit(element, "#prompt-preset-name", "Bundled draft");
    await browse(element, "mine");
    await edit(element, "#prompt-preset-name", "Custom draft");
    element.acceptResetPresets({ ...collection(), revision: 2 });
    await element.updateComplete;
    expect(element.querySelector<HTMLInputElement>("#prompt-preset-name")!.value).toBe(
      "Visual Learner",
    );
    expect(element.querySelector<HTMLSelectElement>("#prompt-preset-list")!.value).toBe(
      "visual-learner",
    );
    expect(element.textContent).not.toContain("Custom draft");
  });
  it("does not roll back a newer snapshot when the restore response arrives late", async () => {
    const { element } = await mount();
    const newer = collection();
    newer.revision = 5;
    newer.presets[0] = { ...newer.presets[0]!, revision: 4, name: "Saved after restore" };
    element.promptPresets = newer;
    await element.updateComplete;
    element.acceptResetPresets({ ...collection(), revision: 3 });
    await element.updateComplete;
    expect(element.promptPresets.revision).toBe(5);
    expect(element.querySelector<HTMLInputElement>("#prompt-preset-name")!.value).toBe(
      "Saved after restore",
    );
  });
  it("keeps request variants editable in Advanced without changing primary instructions", async () => {
    const { element, intents } = await mount();
    expect(element.querySelector<HTMLDetailsElement>(".prompt-advanced")!.open).toBe(false);
    await element.updateComplete;
    await edit(element, "#prompt-request-editor", "Updated initial request.");
    expect(element.querySelector<HTMLTextAreaElement>("#prompt-editor")!.value).toBe(
      template.common,
    );
    button(element, "Save Preset").click();
    expect(intents.at(-1)).toMatchObject({
      change: {
        type: "update",
        template: { ...template, full_projection: "Updated initial request." },
      },
    });
  });
  it("keeps dirty drafts pinned to their preset across tray selection and editor navigation", async () => {
    const { element, intents } = await mount();
    await edit(element, "#prompt-editor", "My draft. {turn_instruction}");
    element.promptPresets = { ...collection(), revision: 2, selected_id: "practical-learner" };
    await element.updateComplete;
    expect(element.querySelector<HTMLTextAreaElement>("#prompt-editor")!.value).toBe(
      "My draft. {turn_instruction}",
    );
    expect(element.querySelector<HTMLSelectElement>("#prompt-preset-list")!.value).toBe(
      "visual-learner",
    );
    await browse(element, "practical-learner");
    await browse(element, "visual-learner");
    button(element, "Save Preset").click();
    expect(intents.at(-1)).toMatchObject({
      type: "presets",
      change: {
        type: "update",
        id: "visual-learner",
        expected_revision: 1,
        template: { common: "My draft. {turn_instruction}" },
      },
    });
  });
  it("browsing does not select and Use selects saved content independently of the draft", async () => {
    const { element, intents } = await mount();
    await browse(element, "practical-learner");
    expect(intents).toEqual([]);
    await edit(element, "#prompt-preset-name", "Unsaved name");
    button(element, "Use This Preset").click();
    expect(intents).toEqual([
      { type: "presets", change: { type: "select", id: "practical-learner" } },
    ]);
  });
  it("preserves conflicts and sends original revision instead of silently overwriting", async () => {
    const { element, intents } = await mount();
    await edit(element, "#prompt-preset-name", "My name");
    const newer = collection();
    newer.presets[0] = { ...newer.presets[0]!, revision: 2, name: "Other editor" };
    element.promptPresets = newer;
    await element.updateComplete;
    expect(element.querySelector('[role="alert"]')?.textContent).toContain(
      "changed or deleted elsewhere",
    );
    button(element, "Save Preset").click();
    expect(intents.at(-1)).toMatchObject({ change: { expected_revision: 1, name: "My name" } });
    button(element, "Reload Saved Preset").click();
    await element.updateComplete;
    expect(element.querySelector<HTMLInputElement>("#prompt-preset-name")!.value).toBe(
      "Other editor",
    );
  });
  it("deletes without a replacement choice and supports restoring a deleted bundled preset", async () => {
    const { element, intents } = await mount();
    button(element, "Delete…").click();
    await element.updateComplete;
    expect(element.querySelector("#prompt-preset-replacement")).toBeNull();
    expect(intents.at(-1)).toMatchObject({
      change: {
        type: "delete",
        id: "visual-learner",
        expected_revision: 1,
      },
    });
    button(element, "Reset All Presets…").click();
    expect(intents.at(-1)).toMatchObject({
      change: { type: "reset_all", expected_catalog_revision: 1 },
    });
    button(element, "Duplicate").click();
    expect(intents.at(-1)).toMatchObject({
      change: { type: "create", name: "Visual Learner Copy", template },
    });
  });
  it("keeps deleted dirty content recoverable as a new preset", async () => {
    const { element, intents } = await mount();
    await edit(element, "#prompt-editor", "Keep me. {turn_instruction}");
    const next = collection();
    next.presets = next.presets.slice(1);
    next.selected_id = "practical-learner";
    element.promptPresets = next;
    await element.updateComplete;
    expect(button(element, "Save Preset").disabled).toBe(true);
    button(element, "Duplicate").click();
    expect(intents.at(-1)).toMatchObject({
      change: { type: "create", template: { common: "Keep me. {turn_instruction}" } },
    });
  });
  it("opens newly created content without selecting it or discarding another draft", async () => {
    const { element } = await mount();
    await edit(element, "#prompt-preset-name", "Keep this draft");
    const next = collection();
    next.revision = 2;
    next.presets.push({ id: "new-id", name: "New Preset", revision: 1, template });
    element.openCreatedPreset(next, "new-id");
    await element.updateComplete;
    expect(element.querySelector<HTMLSelectElement>("#prompt-preset-list")!.value).toBe("new-id");
    expect(element.promptPresets?.selected_id).toBe("visual-learner");
    await browse(element, "visual-learner");
    expect(element.querySelector<HTMLInputElement>("#prompt-preset-name")!.value).toBe(
      "Keep this draft",
    );
  });
  it("retains a deleted draft when browsing away and returning", async () => {
    const { element } = await mount();
    await edit(element, "#prompt-editor", "Retained. {turn_instruction}");
    const next = collection();
    next.presets = next.presets.slice(1);
    next.selected_id = "practical-learner";
    element.promptPresets = next;
    await element.updateComplete;
    await browse(element, "practical-learner");
    await browse(element, "visual-learner");
    expect(element.querySelector<HTMLTextAreaElement>("#prompt-editor")!.value).toBe(
      "Retained. {turn_instruction}",
    );
    expect(button(element, "Save Preset").disabled).toBe(true);
    expect(button(element, "Delete…").disabled).toBe(true);
  });
});
