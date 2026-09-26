// @vitest-environment jsdom
import { afterEach, beforeAll, expect, it } from "vitest";
import { LensTargetSelectionView } from "./lens-target-selection-view";
import "./lens-target-card";
import type { TargetSelectionIntent } from "./events";

beforeAll(() => {
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
});
afterEach(() => document.body.replaceChildren());

async function mount(pending = false) {
  const element = new LensTargetSelectionView();
  element.model = {
    platform: "macos",
    lens: {
      operation_id: "operation",
      stage: "selecting",
      prompt_execution_revision: 1,
      output_blocks: [],
      selection: {
        selection_id: "operation",
        stage: "reviewing",
        maximum_targets: 4,
        items: [
          {
            id: "target",
            window: {
              window_id: 1,
              title: "Example",
              application_name: "Fixture",
              bundle_id: "fixture",
              pid: 1,
              frame: { x: 0, y: 0, width: 100, height: 100 },
            },
          },
        ],
      },
    },
    pending,
    message: "",
  };
  const intents: TargetSelectionIntent[] = [];
  element.addEventListener("lens-target-selection-intent", (event) =>
    intents.push((event as CustomEvent<TargetSelectionIntent>).detail),
  );
  document.body.append(element);
  await element.updateComplete;
  return {
    element,
    intents,
    form: element.shadowRoot!.querySelector<HTMLFormElement>("form")!,
    confirm: element.shadowRoot!.querySelector<HTMLButtonElement>('button[type="submit"]')!,
  };
}

it("commits the selected set once through form submission and preserves independent actions", async () => {
  const { element, intents, form, confirm } = await mount();
  expect(confirm.dataset.lensButtonRole).toBe("primary");
  confirm.click();
  expect(intents).toEqual([{ type: "confirm" }]);
  form.requestSubmit(confirm);
  expect(intents).toEqual([{ type: "confirm" }, { type: "confirm" }]);
  intents.length = 0;
  element
    .shadowRoot!.querySelector<HTMLButtonElement>('[aria-label="Add Another Window"]')!
    .click();
  expect(intents).toEqual([{ type: "add" }]);
  intents.length = 0;
  element.shadowRoot!.querySelector<HTMLButtonElement>('[aria-label^="Remove Fixture"]')!.click();
  expect(intents).toEqual([{ type: "remove", targetId: "target" }]);
  form.requestSubmit(confirm);
  expect(intents).toHaveLength(1);
});

it.each(["pending", "picking", "empty", "missing-operation", "missing-selection"] as const)(
  "rejects even explicit form submission while %s",
  async (state) => {
    const { element, intents, form, confirm } = await mount();
    const model = structuredClone(element.model!);
    if (state === "pending") model.pending = true;
    if (state === "picking") model.lens.selection!.stage = "picking";
    if (state === "empty") model.lens.selection!.items = [];
    if (state === "missing-operation") delete model.lens.operation_id;
    if (state === "missing-selection") delete model.lens.selection;
    element.model = model;
    await element.updateComplete;
    expect(confirm.disabled).toBe(true);
    form.requestSubmit(confirm);
    expect(intents).toEqual([]);
  },
);

it("routes background Return through the intended submitter without a global shortcut", async () => {
  const { element, intents, confirm } = await mount();
  const event = new KeyboardEvent("keydown", {
    key: "Enter",
    bubbles: true,
    composed: true,
    cancelable: true,
  });
  element.dispatchEvent(event);
  expect(event.defaultPrevented).toBe(true);
  expect(intents).toEqual([{ type: "confirm" }]);
  intents.length = 0;
  document.body.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
  );
  expect(intents).toEqual([]);
  confirm.focus();
  element.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
  );
  expect(intents).toEqual([]);
});

it.each([
  { repeat: true },
  { isComposing: true },
  { altKey: true },
  { ctrlKey: true },
  { metaKey: true },
  { shiftKey: true },
  { key: " " },
])("preserves modified, composing, repeated or non-Return keys: %j", async (options) => {
  const { element, intents } = await mount();
  element.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      composed: true,
      cancelable: true,
      ...options,
    }),
  );
  expect(intents).toEqual([]);
});

it.each(["button", "input", "textarea", "select", "lens-select", "a", "div"])(
  "does not steal Enter from a focused %s",
  async (tag) => {
    const { intents, form } = await mount();
    const control = document.createElement(tag);
    if (tag === "a") control.setAttribute("href", "#");
    if (tag === "div") control.setAttribute("contenteditable", "true");
    form.append(control);
    control.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        composed: true,
        cancelable: true,
      }),
    );
    expect(intents).toEqual([]);
  },
);

it("respects an already handled Return", async () => {
  const { element, intents } = await mount();
  const event = new KeyboardEvent("keydown", { key: "Enter", cancelable: true });
  event.preventDefault();
  element.dispatchEvent(event);
  expect(intents).toEqual([]);
});

it("focuses the ready confirmation surface once without adding a Tab stop", async () => {
  const { element, form } = await mount();
  expect(element.shadowRoot!.activeElement).toBe(form);
  expect(form.tabIndex).toBe(-1);
  form.blur();
  element.model = { ...element.model!, message: "Unrelated update" };
  await element.updateComplete;
  expect(element.shadowRoot!.activeElement).toBeNull();
});

it.each(["button", "input"])(
  "does not steal %s focus when selection first becomes usable",
  async (tag) => {
    const { element, form } = await mount(true);
    expect(element.shadowRoot!.activeElement).toBeNull();
    const control = document.createElement(tag);
    form.append(control);
    control.focus();
    element.model = { ...element.model!, pending: false };
    await element.updateComplete;
    expect(element.shadowRoot!.activeElement).toBe(control);
    control.blur();
    element.model = { ...element.model!, message: "Later update" };
    await element.updateComplete;
    expect(element.shadowRoot!.activeElement).toBeNull();
  },
);
