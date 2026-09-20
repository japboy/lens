// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { LensAgentSettings } from "./lens-agent-settings";
import type { AgentIntent } from "./events";
import type { AgentSelectionState } from "../types";

afterEach(() => document.body.replaceChildren());
async function mount(stage: AgentSelectionState["stage"] = "selected") {
  const element = new LensAgentSettings();
  element.selection = {
    stage,
    candidate: { external: "profile-1" },
    supports_logout: false,
    auth_methods: [],
  };
  element.runtime = { stage: "ready", downloaded_bytes: 0 };
  element.profiles = [{ id: "profile-1", name: "Goose", command: "goose", args: ["acp"] }];
  const intents: AgentIntent[] = [];
  element.addEventListener("lens-agent-intent", (event) =>
    intents.push((event as CustomEvent<AgentIntent>).detail),
  );
  document.body.append(element);
  await element.updateComplete;
  return { element, intents };
}
function button(element: LensAgentSettings, text: string) {
  return [...element.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === text,
  )!;
}
function command(element: LensAgentSettings) {
  return element.querySelector<HTMLInputElement>('[aria-label="ACP command"]')!;
}
async function edit(element: LensAgentSettings, value: string) {
  command(element).value = value;
  command(element).dispatchEvent(new Event("input"));
  await element.updateComplete;
}
async function choose(element: LensAgentSettings, value: string) {
  const select = element.querySelector<HTMLSelectElement>('[aria-label="Agent"]')!;
  select.value = value;
  select.dispatchEvent(new Event("change"));
  await element.updateComplete;
}
function browse(element: LensAgentSettings, intents: AgentIntent[]) {
  button(element, "Browse…").click();
  const intent = intents.at(-1)!;
  if (intent.type !== "choose-external-executable") throw new Error();
  return intent.draftRevision;
}

it("uses one Agent menu and keeps managed agents non-deletable", async () => {
  const { element } = await mount();
  expect(element.querySelectorAll("select")).toHaveLength(1);
  expect(element.querySelector('input[type="radio"]')).toBeNull();
  expect(command(element).value).toBe("goose acp");
  expect(button(element, "Delete agent")).toBeDefined();
  await choose(element, "claude");
  expect(command(element)).toBeNull();
  expect(button(element, "Delete agent")).toBeUndefined();
  await choose(element, "codex");
  expect(button(element, "Delete agent")).toBeUndefined();
});
it.each(["failed", "history_selected", "unselected"] as const)(
  "shows the external candidate at %s without claiming readiness",
  async (stage) => {
    const { element } = await mount(stage);
    expect(element.querySelector<HTMLSelectElement>('[aria-label="Agent"]')!.value).toBe(
      "profile-1",
    );
    expect(element.querySelector(".status-ok")).toBeNull();
    expect(command(element)).not.toBeNull();
  },
);
it("sends the raw command only on Save and Verify and preserves arguments when browsing", async () => {
  const { element, intents } = await mount();
  await edit(element, `goose acp '' ' literal argument '`);
  expect(intents).toEqual([]);
  expect(element.textContent).toContain("Unsaved connection changes");
  const revision = browse(element, intents);
  expect(element.acceptExternalExecutable("/chosen path/goose", revision)).toBe(true);
  await element.updateComplete;
  expect(command(element).value).toBe(`'/chosen path/goose' acp '' ' literal argument '`);
  button(element, "Save and Verify").click();
  expect(intents.at(-1)).toEqual({
    type: "save-external-agent",
    profile: {
      id: "profile-1",
      name: "Goose",
      command_line: `'/chosen path/goose' acp '' ' literal argument '`,
    },
  });
  expect(element.profiles[0]!.command).toBe("goose");
});
it("keeps invalid quoted text local and cannot save or overwrite it with Browse", async () => {
  const { element, intents } = await mount();
  await edit(element, "goose 'incomplete");
  expect(element.querySelector('[role="alert"]')).not.toBeNull();
  expect(button(element, "Save and Verify").disabled).toBe(true);
  const revision = browse(element, intents);
  expect(element.acceptExternalExecutable("/chosen/goose", revision)).toBe(false);
  expect(command(element).value).toBe("goose 'incomplete");
});
it("retains drafts across managed switches and equivalent snapshots", async () => {
  const { element } = await mount();
  await edit(element, "goose acp --example");
  await choose(element, "claude");
  await choose(element, "profile-1");
  element.profiles = structuredClone(element.profiles);
  await element.updateComplete;
  expect(command(element).value).toBe("goose acp --example");
});
it.each(["typing", "saved-args", "agent-switch", "new-browse", "disconnect"])(
  "invalidates delayed Browse after %s",
  async (change) => {
    const { element, intents } = await mount();
    const revision = browse(element, intents);
    if (change === "typing") await edit(element, "goose acp --new");
    if (change === "saved-args") {
      element.profiles = [{ ...element.profiles[0]!, args: ["acp", ""] }];
      await element.updateComplete;
    }
    if (change === "agent-switch") {
      await choose(element, "claude");
      await choose(element, "profile-1");
    }
    if (change === "new-browse") browse(element, intents);
    if (change === "disconnect") element.remove();
    expect(element.acceptExternalExecutable("/late/goose", revision)).toBe(false);
  },
);
it("adds stable UUID drafts, separates duplicate names, and deletes by ID", async () => {
  const { element, intents } = await mount();
  button(element, "Add agent").click();
  await element.updateComplete;
  const name = element.querySelector<HTMLInputElement>('[aria-label="Connection name"]')!;
  name.value = "Goose";
  name.dispatchEvent(new Event("input"));
  await edit(element, "other-acp --stdio");
  button(element, "Save and Verify").click();
  const intent = intents.at(-1)!;
  if (intent.type !== "save-external-agent") throw new Error();
  expect(intent.profile.id).toMatch(/^[\da-f-]{36}$/);
  const id = intent.profile.id;
  button(element, "Save and Verify").click();
  expect(intents.at(-1)).toEqual(intent);
  element.profiles = [
    ...element.profiles,
    { id, name: "Goose", command: "other-acp", args: ["--stdio"] },
  ];
  await element.updateComplete;
  expect(
    [...element.querySelectorAll("option")].filter((option) =>
      option.textContent?.includes("Goose"),
    ),
  ).toHaveLength(2);
  button(element, "Delete agent").click();
  expect(intents.at(-1)).toEqual({ type: "delete-external-agent", id });
});
it("disables editing while pending and uses advertised logout capability", async () => {
  const { element } = await mount();
  element.disabled = true;
  await element.updateComplete;
  expect(command(element).closest("fieldset")!.disabled).toBe(true);
  element.disabled = false;
  element.selection = { ...element.selection!, supports_logout: true };
  await element.updateComplete;
  expect(button(element, "Sign Out…")).toBeDefined();
});
it("lets a history-selected managed agent verify its connection", async () => {
  const { element, intents } = await mount("history_selected");
  element.selection = { ...element.selection!, candidate: "claude" };
  await element.updateComplete;
  button(element, "Verify connection").click();
  expect(intents.at(-1)).toEqual({ type: "select", agent: "claude" });
});
it("accepts Browse for an empty new command draft", async () => {
  const { element, intents } = await mount();
  button(element, "Add agent").click();
  await element.updateComplete;
  const revision = browse(element, intents);
  expect(element.acceptExternalExecutable("/new path/agent", revision)).toBe(true);
  await element.updateComplete;
  expect(command(element).value).toBe("'/new path/agent'");
});
it("blocks saving a legacy multiline argument instead of silently stripping it", async () => {
  const { element } = await mount();
  element.profiles = [{ ...element.profiles[0]!, args: ["line\nbreak"] }];
  await element.updateComplete;
  expect(element.querySelector('[role="alert"]')!.textContent).toContain("single-line");
  expect(button(element, "Save and Verify").disabled).toBe(true);
});

it("renders both first-run external presets with managed agents in the single selector", async () => {
  // Matches ExternalAgentProfile::{goose_preset,copilot_preset} used by AppConfig::new.
  const presets = [
    { id: "6b57315e-9c13-4e4a-bf4c-e6bc33b10b21", name: "Goose", command: "goose", args: ["acp"] },
    {
      id: "a14d73cb-951c-48ed-a305-3829750c88da",
      name: "GitHub Copilot",
      command: "copilot",
      args: ["--acp", "--stdio"],
    },
  ];
  const element = new LensAgentSettings();
  element.selection = { stage: "unselected", supports_logout: false, auth_methods: [] };
  element.runtime = { stage: "not_installed", downloaded_bytes: 0 };
  element.profiles = presets;
  const intents: AgentIntent[] = [];
  element.addEventListener("lens-agent-intent", (event) =>
    intents.push((event as CustomEvent<AgentIntent>).detail),
  );
  document.body.append(element);
  await element.updateComplete;
  expect(element.querySelectorAll("select")).toHaveLength(1);
  const selector = element.querySelector<HTMLSelectElement>('[aria-label="Agent"]')!;
  expect([...selector.options].map((option) => option.textContent?.trim())).toEqual([
    "Claude",
    "Codex",
    "Goose",
    "GitHub Copilot",
  ]);
  expect(intents).toEqual([]);
  await choose(element, presets[1]!.id);
  expect(selector.value).toBe(presets[1]!.id);
  expect(command(element).value).toBe("copilot --acp --stdio");
  expect(intents).toEqual([{ type: "select", agent: { external: presets[1]!.id } }]);
  expect(button(element, "Delete agent")).toBeDefined();
});

it("resets saved edits and unsaved drafts only after acceptance and invalidates pending Browse", async () => {
  const { element, intents } = await mount();
  await edit(element, "goose acp --edited");
  button(element, "Add agent").click();
  await element.updateComplete;
  await edit(element, "custom-agent");
  const revision = browse(element, intents);
  button(element, "Reset Agent Presets…").click();
  expect(intents.at(-1)).toEqual({ type: "reset-agent-presets" });
  expect(command(element).value).toBe("custom-agent");
  const count = intents.length;
  element.acceptResetPresets(element.profiles, "claude");
  await element.updateComplete;
  expect(element.acceptExternalExecutable("/late/agent", revision)).toBe(false);
  expect(element.querySelector<HTMLSelectElement>('[aria-label="Agent"]')!.value).toBe("claude");
  expect(element.querySelectorAll("option")).toHaveLength(3);
  expect(intents).toHaveLength(count);
  await choose(element, "profile-1");
  expect(command(element).value).toBe("goose acp");
});

it.each(["unselected", "failed", "authentication_required", "history_selected"] as const)(
  "can explicitly verify the current managed choice at %s without selecting another agent",
  async (stage) => {
    const { element, intents } = await mount(stage);
    element.selection = { ...element.selection!, candidate: "claude" };
    await element.updateComplete;
    expect(intents).toEqual([]);
    button(element, "Verify connection").click();
    expect(intents).toEqual([{ type: "select", agent: "claude" }]);
  },
);
it("returns to the authoritative external choice after discarding a new draft without launching", async () => {
  const { element, intents } = await mount();
  await edit(element, "goose acp --saved-profile-draft");
  button(element, "Add agent").click();
  await element.updateComplete;
  await edit(element, "another-agent");
  button(element, "Discard draft").click();
  await element.updateComplete;
  expect(element.querySelector<HTMLSelectElement>('[aria-label="Agent"]')!.value).toBe("profile-1");
  expect(command(element).value).toBe("goose acp --saved-profile-draft");
  expect(intents).toEqual([]);
});
