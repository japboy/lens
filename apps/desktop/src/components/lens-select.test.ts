// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { LensSelect } from "./lens-select";

afterEach(() => {
  document.body.replaceChildren();
});
async function mount() {
  const element = new LensSelect();
  element.label = "Agent";
  element.value = "uuid-a";
  element.options = [
    { value: "uuid-a", label: "Alpha", icon: "/alpha.svg" },
    { value: "skip", label: "Disabled", disabled: true },
    { value: "uuid-b", label: "Beta", icon: "/beta.svg" },
    { value: "uuid-c", label: "Bravo" },
  ];
  document.body.append(element);
  await element.updateComplete;
  const changed: string[] = [];
  element.addEventListener("change", () => changed.push(element.value));
  const button = element.shadowRoot!.querySelector<HTMLButtonElement>("button")!;
  const key = async (key: string) => {
    button.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    await element.updateComplete;
  };
  return { element, button, key, changed };
}
describe("select-only combobox", () => {
  it("highlights with arrows and commits the UUID only on Enter", async () => {
    const { element, button, key, changed } = await mount();
    await key("ArrowDown");
    await key("ArrowDown");
    expect(button.getAttribute("aria-activedescendant")).toMatch(/-2$/);
    expect(element.value).toBe("uuid-a");
    expect(changed).toEqual([]);
    await key("Enter");
    expect(element.value).toBe("uuid-b");
    expect(changed).toEqual(["uuid-b"]);
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(button.querySelector(".lens-select-icon")?.getAttribute("style")).toContain("/beta.svg");
  });
  it("cancels pending navigation on Escape, Tab and outside pointer without focus theft", async () => {
    const { element, button, key, changed } = await mount();
    for (const closing of ["Escape", "Tab"]) {
      await key("End");
      await key(closing);
      expect(element.value).toBe("uuid-a");
      expect(button.getAttribute("aria-expanded")).toBe("false");
    }
    await key("End");
    const outside = document.createElement("button");
    document.body.append(outside);
    outside.focus();
    outside.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    await element.updateComplete;
    expect(document.activeElement).toBe(outside);
    expect(element.value).toBe("uuid-a");
    expect(changed).toEqual([]);
  });
  it("supports Home End typeahead and Space with explicit highlight", async () => {
    const { element, button, key, changed } = await mount();
    await key("End");
    expect(button.getAttribute("aria-activedescendant")).toMatch(/-3$/);
    await key("Home");
    expect(button.getAttribute("aria-activedescendant")).toMatch(/-0$/);
    await key("b");
    await key("r");
    expect(button.getAttribute("aria-activedescendant")).toMatch(/-3$/);
    await key(" ");
    expect(element.value).toBe("uuid-c");
    expect(changed).toEqual(["uuid-c"]);
  });
  it("cancels when disabled and rejects disabled pointer options", async () => {
    const { element, button, key, changed } = await mount();
    button.click();
    await element.updateComplete;
    element.shadowRoot!.querySelector<HTMLElement>('[data-index="1"]')!.click();
    expect(changed).toEqual([]);
    await key("End");
    element.disabled = true;
    await element.updateComplete;
    expect(button.disabled).toBe(true);
    expect(button.getAttribute("aria-expanded")).toBe("false");
    await key("Enter");
    expect(changed).toEqual([]);
    expect(element.value).toBe("uuid-a");
  });
  it("reconciles replacement options and commits a pointer choice once", async () => {
    const { element, button, key, changed } = await mount();
    await key("End");
    element.options = element.options.slice(0, 3);
    await element.updateComplete;
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(changed).toEqual([]);
    button.click();
    await element.updateComplete;
    element.shadowRoot!.querySelector<HTMLElement>('[data-index="2"]')!.click();
    await element.updateComplete;
    expect(changed).toEqual(["uuid-b"]);
  });
});

it("closes on scroll inside the containing shadow root", async () => {
  const container = document.createElement("div");
  const root = container.attachShadow({ mode: "open" });
  const scrolling = document.createElement("div");
  root.append(scrolling);
  document.body.append(container);
  const element = new LensSelect();
  element.options = [{ value: "a", label: "Alpha" }];
  element.value = "a";
  scrolling.append(element);
  await element.updateComplete;
  const button = element.shadowRoot!.querySelector<HTMLButtonElement>("button")!;
  button.click();
  await element.updateComplete;
  expect(button.getAttribute("aria-expanded")).toBe("true");
  scrolling.dispatchEvent(new Event("scroll", { bubbles: false, composed: false }));
  await element.updateComplete;
  expect(button.getAttribute("aria-expanded")).toBe("false");
});

it("focuses the trigger on pointer opening so arrows and Escape work", async () => {
  const { element, button, changed } = await mount();
  const outside = document.createElement("button");
  document.body.append(outside);
  outside.focus();
  button.click();
  await element.updateComplete;
  expect(element.shadowRoot!.activeElement).toBe(button);
  element.shadowRoot!.activeElement!.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "ArrowDown",
      bubbles: true,
      cancelable: true,
    }),
  );
  await element.updateComplete;
  expect(button.getAttribute("aria-activedescendant")).toMatch(/-2$/);
  element.shadowRoot!.activeElement!.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "Escape",
      bubbles: true,
      cancelable: true,
    }),
  );
  await element.updateComplete;
  expect(button.getAttribute("aria-expanded")).toBe("false");
  expect(element.value).toBe("uuid-a");
  expect(changed).toEqual([]);
});

it("toggles multiple choices without modifiers, serializes them, and closes on Enter or Done", async () => {
  const { element, button, key } = await mount();
  const form = document.createElement("form");
  document.body.append(form);
  form.append(element);
  element.name = "choices";
  element.multiple = true;
  element.values = ["uuid-a"];
  await element.updateComplete;
  await key("End");
  await key(" ");
  expect(element.values).toEqual(["uuid-a", "uuid-c"]);
  expect(button.getAttribute("aria-expanded")).toBe("true");
  expect(
    element.shadowRoot!.querySelector('[role="listbox"]')?.getAttribute("aria-multiselectable"),
  ).toBe("true");
  expect(element.formEntries()).toEqual([
    ["choices", "uuid-a"],
    ["choices", "uuid-c"],
  ]);
  await key("Enter");
  expect(button.getAttribute("aria-expanded")).toBe("false");
  expect(element.values).toEqual(["uuid-a", "uuid-c"]);
  button.click();
  await element.updateComplete;
  element.shadowRoot!.querySelector<HTMLElement>('[data-index="0"]')!.click();
  await element.updateComplete;
  expect(element.values).toEqual(["uuid-c"]);
  element.shadowRoot!.querySelector<HTMLButtonElement>(".lens-select-done")!.click();
  await element.updateComplete;
  expect(button.getAttribute("aria-expanded")).toBe("false");
  expect(element.shadowRoot!.activeElement).toBe(button);
});

it("validates required choices, excludes disabled controls, and never substitutes replacement options", async () => {
  const { element, button, key } = await mount();
  const form = document.createElement("form");
  const fieldset = document.createElement("fieldset");
  document.body.append(form);
  form.append(fieldset);
  fieldset.append(element);
  element.name = "choices";
  element.multiple = true;
  element.required = true;
  element.values = [];
  await element.updateComplete;
  expect(element.reportValidity()).toBe(false);
  await element.updateComplete;
  expect(button.getAttribute("aria-invalid")).toBe("true");
  expect(element.formEntries()).toEqual([]);
  element.disabled = true;
  element.values = ["uuid-a"];
  await element.updateComplete;
  expect(element.reportValidity()).toBe(true);
  expect(element.formEntries()).toEqual([]);
  element.disabled = false;
  await element.updateComplete;
  fieldset.disabled = true;
  await key("End");
  expect(button.getAttribute("aria-expanded")).toBe("false");
  expect(element.formEntries()).toEqual([]);
  fieldset.disabled = false;
  expect(element.reportValidity()).toBe(true);
  element.options = [{ value: "replacement", label: "Replacement" }];
  await element.updateComplete;
  expect(element.values).toEqual(["uuid-a"]);
  expect(element.reportValidity()).toBe(false);
  expect(element.formEntries()).toEqual([]);
  expect(button.textContent).toContain("Choose…");
});

it("sizes the popup to its labels and clamps a narrow trigger near the viewport edge", async () => {
  const { element, button, key } = await mount();
  element.options = [
    { value: "license", label: "LICENSE" },
    { value: "notice", label: "NOTICE" },
  ];
  element.value = "license";
  await element.updateComplete;
  const viewportWidth = window.innerWidth;
  vi.spyOn(button, "getBoundingClientRect").mockReturnValue({
    left: viewportWidth - 99,
    top: 100,
    bottom: 124,
    width: 91,
  } as DOMRect);
  const popup = element.shadowRoot!.querySelector<HTMLElement>(".lens-select-popup")!;
  vi.spyOn(popup, "getBoundingClientRect").mockReturnValue({ width: 160 } as DOMRect);
  await key("ArrowDown");
  await element.updateComplete;
  expect(popup.style.width).toBe("max-content");
  expect(popup.style.minWidth).toBe("91px");
  expect(popup.style.maxWidth).toBe(`${viewportWidth - 16}px`);
  expect(popup.style.left).toBe(`${viewportWidth - 168}px`);
  await key("ArrowDown");
  await key("Enter");
  expect(element.value).toBe("notice");
});
