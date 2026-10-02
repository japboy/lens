// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { LensMcpAppSettings } from "./lens-mcp-app-settings";
import type { LensSelect } from "./lens-select";
import { SETTINGS_INTENT_EVENT } from "./events";
const first = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "Original",
  url: "https://example.com/mcp",
};
const second = {
  id: "22222222-2222-4222-8222-222222222222",
  name: "Other",
  url: "https://other.example/mcp",
};
afterEach(() => document.body.replaceChildren());
async function mount(servers = [first, second]) {
  const element = new LensMcpAppSettings();
  element.servers = servers;
  document.body.append(element);
  await element.updateComplete;
  const intents: unknown[] = [];
  element.addEventListener(SETTINGS_INTENT_EVENT, (event) =>
    intents.push((event as CustomEvent).detail),
  );
  return { element, intents };
}
async function select(element: LensMcpAppSettings, value: string) {
  const selector = element.querySelector<LensSelect>("lens-select")!;
  selector.value = value;
  selector.dispatchEvent(new Event("change", { bubbles: true }));
  await element.updateComplete;
}
function button(element: LensMcpAppSettings, label: string) {
  return [...element.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) => item.textContent?.trim() === label,
  )!;
}
async function input(element: LensMcpAppSettings, label: string, value: string) {
  const control = element.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
  control.value = value;
  control.dispatchEvent(new Event("input", { bubbles: true }));
  await element.updateComplete;
}
async function save(element: LensMcpAppSettings) {
  element
    .querySelector("form")!
    .dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await element.updateComplete;
}
describe("MCP preset settings", () => {
  it("starts with only the permanent built-in and does not register example values", async () => {
    const { element, intents } = await mount([]);
    const selector = element.querySelector<LensSelect>("lens-select")!;
    expect(selector.options).toEqual([
      { value: "lens_rich_content", label: "lens_rich_content · Built-in" },
    ]);
    expect(element.querySelector("h2")!.textContent).toBe("MCP");
    expect(element.textContent).toContain("render_html");
    expect(button(element, "Delete Preset")).toBeUndefined();
    expect(button(element, "Save")).toBeUndefined();
    button(element, "Add Preset").click();
    await element.updateComplete;
    const serverSummary = element.querySelector(".mcp-preset-summary dd")!;
    expect(serverSummary.textContent?.trim()).toBe("Not registered");
    expect(serverSummary.querySelector("code")).toBeNull();
    const controls = element.querySelectorAll("input");
    expect([...controls].map((item) => item.value)).toEqual(["", ""]);
    expect([...controls].map((item) => item.placeholder)).toEqual([
      "Reference",
      "https://mcp.example.com/mcp",
    ]);
    button(element, "Discard Draft").click();
    await element.updateComplete;
    expect(element.querySelector("form")).toBeNull();
    expect(intents).toEqual([]);
  });

  it("retains a new unsaved draft across built-in selection through the combobox", async () => {
    const { element, intents } = await mount([]);
    button(element, "Add Preset").click();
    await element.updateComplete;
    await input(element, "MCP preset name", "validation-draft");
    await input(element, "Streamable HTTP URL", "https://example.com/mcp");
    const selector = element.querySelector<LensSelect>("lens-select")!;
    await selector.updateComplete;
    const draftId = selector.value;
    const choose = async (value: string) => {
      const trigger = selector.shadowRoot!.querySelector<HTMLButtonElement>("button")!;
      trigger.click();
      await selector.updateComplete;
      expect(trigger.getAttribute("aria-expanded")).toBe("true");
      const index = selector.options.findIndex((option) => option.value === value);
      expect(index).toBeGreaterThanOrEqual(0);
      selector.shadowRoot!.querySelector<HTMLElement>(`[data-index="${index}"]`)!.click();
      await element.updateComplete;
      await selector.updateComplete;
      expect(selector.value).toBe(value);
      expect(trigger.getAttribute("aria-expanded")).toBe("false");
    };
    await choose("lens_rich_content");
    expect(element.querySelector("form")).toBeNull();
    element.servers = [];
    await element.updateComplete;
    await selector.updateComplete;
    expect(selector.options).toEqual([
      { value: "lens_rich_content", label: "lens_rich_content · Built-in" },
      { value: draftId, label: "validation-draft (unsaved)" },
    ]);
    await choose(draftId);
    expect(element.querySelector<HTMLInputElement>('[aria-label="MCP preset name"]')!.value).toBe(
      "validation-draft",
    );
    expect(
      element.querySelector<HTMLInputElement>('[aria-label="Streamable HTTP URL"]')!.value,
    ).toBe("https://example.com/mcp");
    expect(button(element, "Discard Draft")).toBeDefined();
    expect(button(element, "Delete Preset")).toBeUndefined();
    expect(intents).toEqual([]);
  });

  it("retains unsaved text across equal snapshots and selection and saves the entire registry", async () => {
    const { element, intents } = await mount();
    await select(element, first.id);
    await input(element, "Streamable HTTP URL", "https://example.com/new-mcp");
    element.servers = element.servers.map((server) => ({ ...server }));
    await element.updateComplete;
    await select(element, second.id);
    await select(element, first.id);
    expect(
      element.querySelector<HTMLInputElement>('input[aria-label="Streamable HTTP URL"]')!.value,
    ).toBe("https://example.com/new-mcp");
    expect(intents).toEqual([]);
    await save(element);
    expect(intents).toEqual([
      {
        type: "set-mcp-apps-servers",
        servers: [{ ...first, url: "https://example.com/new-mcp" }, second],
      },
    ]);
    element.servers = [{ ...first, url: "https://example.com/saved" }, second];
    await element.updateComplete;
    expect(
      element.querySelector<HTMLInputElement>('input[aria-label="Streamable HTTP URL"]')!.value,
    ).toBe("https://example.com/saved");
  });

  it("adds only the reviewed draft, retains other registrations, and deletes only the selected external preset", async () => {
    const { element, intents } = await mount();
    button(element, "Add Preset").click();
    await element.updateComplete;
    await input(element, "MCP preset name", " Third ");
    await input(element, "Streamable HTTP URL", " https://third.example/mcp ");
    await save(element);
    expect(intents[0]).toMatchObject({
      type: "set-mcp-apps-servers",
      servers: [first, second, { name: "Third", url: "https://third.example/mcp" }],
    });
    await select(element, first.id);
    button(element, "Delete Preset").click();
    await element.updateComplete;
    expect(intents[1]).toEqual({ type: "set-mcp-apps-servers", servers: [second] });
    expect(element.servers).toEqual([first, second]);
  });

  it("keeps saved preset actions distinct from unsaved draft actions", async () => {
    const { element, intents } = await mount();
    await select(element, first.id);
    await input(element, "MCP preset name", "Changed");
    expect(button(element, "Delete Preset")).toBeDefined();
    expect(button(element, "Discard Draft")).toBeUndefined();
    expect(button(element, "Save").disabled).toBe(false);
    expect(
      element
        .querySelector("form")!
        .compareDocumentPosition(element.querySelector(".mcp-preset-summary")!) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    button(element, "Add Preset").click();
    await element.updateComplete;
    expect(button(element, "Delete Preset")).toBeUndefined();
    expect(button(element, "Discard Draft")).toBeDefined();
    expect(intents).toEqual([]);
  });

  it("preserves drafts until an accepted reset clears the complete MCP editor", async () => {
    const { element, intents } = await mount();
    await select(element, first.id);
    await input(element, "MCP preset name", "Changed");
    button(element, "Add Preset").click();
    await element.updateComplete;
    await input(element, "MCP preset name", "Unsaved");
    button(element, "Reset Presets…").click();
    await element.updateComplete;
    expect(intents).toEqual([{ type: "reset-mcp-presets" }]);
    expect(
      element.querySelector<HTMLInputElement>('input[aria-label="MCP preset name"]')!.value,
    ).toBe("Unsaved");
    element.acceptResetPresets();
    await element.updateComplete;
    expect(element.servers).toEqual([]);
    expect(element.querySelector<LensSelect>("lens-select")!.options).toEqual([
      { value: "lens_rich_content", label: "lens_rich_content · Built-in" },
    ]);
    expect(element.querySelector("form")).toBeNull();
    expect(element.querySelector(".mcp-preset-summary")!.textContent).toContain("render_html");
  });

  it("distinguishes unloaded, empty and source-matched cached tool catalogs", async () => {
    const { element } = await mount();
    await select(element, first.id);
    const tools = () => element.querySelector(".mcp-preset-summary")!.textContent;
    expect(tools()).toContain("Not loaded");
    element.catalogs = [{ server: { ...first }, tools: [] }];
    await element.updateComplete;
    expect(tools()).toContain("No tools");
    element.catalogs = [{ server: { ...first }, tools: ["ordinary_tool", "render_chart"] }];
    await element.updateComplete;
    expect(tools()).toContain("ordinary_tool");
    expect(tools()).toContain("render_chart");
    element.servers = [{ ...first, url: "https://replacement.example/mcp" }, second];
    await element.updateComplete;
    expect(tools()).toContain("Not loaded");
    expect(tools()).not.toContain("ordinary_tool");
  });

  it.each([
    "",
    "has space",
    "\u65e5\u672c\u8a9e",
    "a".repeat(65),
    "Other",
    "lens_rich_content",
    "lens_output",
  ])("rejects invalid or conflicting source name %s without losing the draft", async (name) => {
    const { element, intents } = await mount();
    await select(element, first.id);
    await input(element, "MCP preset name", name);
    await save(element);
    expect(intents).toEqual([]);
    expect(element.querySelector('[role="alert"]')).not.toBeNull();
    expect(
      element.querySelector<HTMLInputElement>('input[aria-label="MCP preset name"]')!.value,
    ).toBe(name);
  });

  it.each([
    "stdio://command",
    "not a URL",
    "https://user:secret@example.com/mcp",
    "https://example.com/mcp?token=x",
    "https://example.com/mcp?",
    "https://example.com/mcp#fragment",
    "https://example.com/mcp#",
  ])("rejects unsupported endpoint %s", async (url) => {
    const { element, intents } = await mount();
    await select(element, first.id);
    await input(element, "Streamable HTTP URL", url);
    await save(element);
    expect(intents).toEqual([]);
    expect(element.querySelector('[role="alert"]')).not.toBeNull();
  });

  it("limits external presets to sixteen and prevents disabled submission", async () => {
    const servers = Array.from({ length: 16 }, (_, index) => ({
      id: `00000000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
      name: `server_${index}`,
      url: "http://localhost/mcp",
    }));
    const { element, intents } = await mount(servers);
    expect(button(element, "Add Preset").disabled).toBe(true);
    await select(element, servers[0]!.id);
    await input(element, "MCP preset name", "Changed");
    element.disabled = true;
    await element.updateComplete;
    await save(element);
    button(element, "Delete Preset").click();
    expect(intents).toEqual([]);
  });
});
