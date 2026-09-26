// @vitest-environment jsdom
import type { LensSelect } from "./lens-select";
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
  await element.querySelector<LensSelect>("lens-select")?.updateComplete;
  return { element, intents };
}
function button(element: LensAgentSettings, text: string) {
  return [...element.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === text,
  )!;
}
function command(element: LensAgentSettings) {
  return element.querySelector<HTMLInputElement>('[aria-label="Executable"]')!;
}
function argumentsField(element: LensAgentSettings) {
  return element.querySelector<HTMLInputElement>('[aria-label="Arguments"]')!;
}
async function edit(element: LensAgentSettings, value: string) {
  const [executable, ...args] = value.split(" ");
  command(element).value = executable!;
  command(element).dispatchEvent(new Event("input"));
  argumentsField(element).value = args.join(" ");
  argumentsField(element).dispatchEvent(new Event("input"));
  await element.updateComplete;
}
async function choose(element: LensAgentSettings, value: string) {
  const select = element.querySelector<LensSelect>("lens-select")!;
  await select.updateComplete;
  select.shadowRoot!.querySelector<HTMLButtonElement>("button")!.click();
  await select.updateComplete;
  const index = select.options.findIndex((option) => option.value === value);
  select.shadowRoot!.querySelector<HTMLElement>(`[data-index="${index}"]`)!.click();
  await select.updateComplete;
  await element.updateComplete;
}
function browse(element: LensAgentSettings, intents: AgentIntent[]) {
  button(element, "Choose…").click();
  const intent = intents.at(-1)!;
  if (intent.type !== "choose-external-executable") throw new Error();
  return intent.draftRevision;
}

it("uses one Agent menu and keeps managed agents non-deletable", async () => {
  const { element } = await mount();
  expect(element.querySelectorAll("lens-select")).toHaveLength(1);
  expect(element.querySelector('input[type="radio"]')).toBeNull();
  expect(command(element).value).toBe("goose");
  expect(button(element, "Delete Preset")).toBeDefined();
  await choose(element, "claude");
  expect(command(element)).toBeNull();
  expect(button(element, "Delete Preset")).toBeUndefined();
  await choose(element, "codex");
  expect(button(element, "Delete Preset")).toBeUndefined();
});
it("keeps external ACP editing and runtime status in one Agent card", async () => {
  const { element } = await mount();
  element.runtime = {
    agent: { external: "profile-1" },
    stage: "verifying",
    downloaded_bytes: 0,
    message: "Checking user-owned executable…",
  };
  await element.updateComplete;
  expect(button(element, "Install")).toBeUndefined();
  const cards = element.querySelectorAll<HTMLElement>(":scope > .settings-group");
  expect(cards).toHaveLength(1);
  expect(cards[0]?.querySelector("h2")?.textContent).toBe("Agent");
  const presets = element.querySelector<HTMLElement>(".agent-preset-settings")!;
  expect(presets.closest(".settings-group")).toBe(cards[0]);
  expect(presets.querySelector("legend")?.classList.contains("visually-hidden")).toBe(true);
  expect(button(element, "Add Preset").closest(".preset-add-actions")).not.toBeNull();
  expect(button(element, "Add Preset").closest(".settings-group")).toBe(cards[0]);
  expect(button(element, "Reset Presets…").closest(".agent-reset-actions")).not.toBeNull();
  expect(button(element, "Reset Presets…").closest(".settings-group")).toBe(cards[0]);
  const runtimeStatus = element.querySelector(".runtime-status")!;
  expect(runtimeStatus.closest(".settings-group")).toBe(cards[0]);
  expect(runtimeStatus.textContent).toContain("Checking user-owned executable…");
  expect(presets.compareDocumentPosition(runtimeStatus) & Node.DOCUMENT_POSITION_FOLLOWING).toBe(
    Node.DOCUMENT_POSITION_FOLLOWING,
  );
  expect(
    runtimeStatus.compareDocumentPosition(button(element, "Reset Presets…")) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
});

it("keeps the Agent selector accessible without a duplicate visible label", async () => {
  const { element } = await mount();
  const select = element.querySelector<LensSelect>("lens-select")!;
  expect(select.label).toBe("Agent");
  expect(select.shadowRoot?.querySelector("button")?.getAttribute("aria-label")).toBe("Agent");
  expect(select.parentElement?.querySelector("span")).toBeNull();
});

it("updates the managed Agent shown in the selector and preserves progress", async () => {
  const { element, intents } = await mount();
  element.selection = { ...element.selection!, candidate: "claude" };
  element.runtime = { agent: "claude", stage: "ready", downloaded_bytes: 0 };
  await element.updateComplete;
  expect(element.querySelector<LensSelect>("lens-select")?.value).toBe("claude");
  expect(element.querySelectorAll(".managed-agent-actions button")).toHaveLength(1);
  expect(
    button(element, "Add Preset").compareDocumentPosition(button(element, "Install")) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  button(element, "Install").click();
  expect(intents.at(-1)).toEqual({ type: "update-managed-agent", agent: "claude" });
  expect(element.selection.candidate).toBe("claude");

  await choose(element, "codex");
  expect(intents.at(-1)).toEqual({ type: "select", agent: "codex" });
  element.selection = { ...element.selection!, candidate: "codex" };
  element.runtime = { agent: "codex", stage: "ready", downloaded_bytes: 0 };
  await element.updateComplete;
  expect(element.querySelector<LensSelect>("lens-select")?.value).toBe("codex");
  expect(element.querySelectorAll(".managed-agent-actions button")).toHaveLength(1);
  button(element, "Install").click();
  expect(intents.at(-1)).toEqual({ type: "update-managed-agent", agent: "codex" });
  element.updatePending = true;
  element.updateAgent = "codex";
  await element.updateComplete;
  expect(element.querySelector(".runtime-status")?.textContent).toContain(
    "Checking ChatGPT Codex for updates…",
  );
  element.runtime = {
    agent: "codex",
    stage: "downloading",
    downloaded_bytes: 5,
    total_bytes: 10,
    message: "Downloading Codex…",
  };
  await element.updateComplete;
  expect(button(element, "Install").disabled).toBe(true);
  expect(element.querySelector(".runtime-status")?.textContent).toContain("Downloading Codex…");
  expect(element.querySelector("progress")?.getAttribute("max")).toBe("10");
  element.updatePending = false;
  element.runtime = {
    agent: "codex",
    stage: "ready",
    downloaded_bytes: 0,
    message: "Installed Codex",
  };
  await element.updateComplete;
  expect(element.querySelector(".runtime-status")?.textContent).toContain("Installed Codex");
  expect(element.selection.candidate).toBe("codex");
});

it.each([
  { currentVersion: null, stage: "not_installed", label: "Install" },
  { currentVersion: "1.0.0", stage: "ready", label: "Update" },
  { currentVersion: "1.0.0", stage: "failed", label: "Update" },
  { currentVersion: null, stage: "failed", label: "Install" },
] as const)(
  "shows $label using the confirmed installation at $stage",
  async ({ currentVersion, stage, label }) => {
    const { element } = await mount();
    element.selection = { ...element.selection!, candidate: "codex" };
    element.runtime = {
      agent: "codex",
      current_version: currentVersion,
      version: "2.0.0",
      stage,
      downloaded_bytes: 0,
    };
    await element.updateComplete;
    expect(button(element, label).closest(".installation-status")).not.toBeNull();
    expect(button(element, label).closest(".preset-add-actions")).toBeNull();
  },
);

it("offers verification without save instructions for the initial managed choice", async () => {
  const { element } = await mount("unselected");
  element.selection = { stage: "unselected", supports_logout: false, auth_methods: [] };
  await element.updateComplete;
  expect(element.querySelector(".selection-status")?.textContent).not.toContain("Save");
  expect(button(element, "Verify Connection")).toBeDefined();
});
it("groups preset management and keeps Choose beside the executable", async () => {
  const { element } = await mount();
  expect(button(element, "Delete Preset").closest(".preset-add-actions")).toBe(
    button(element, "Add Preset").closest(".preset-add-actions"),
  );
  expect(button(element, "Choose…").closest(".executable-row")?.contains(command(element))).toBe(
    true,
  );
  expect(
    button(element, "Choose…").closest(".executable-row")?.contains(argumentsField(element)),
  ).toBe(false);
});

it("shows control help on focus and hover and dismisses it with Escape", async () => {
  const { element } = await mount();
  const add = button(element, "Add Preset");
  const help = element.querySelector<HTMLElement>("#agent-add-help")!;
  expect(add.getAttribute("aria-describedby")).toBe(help.id);
  expect(help.hidden).toBe(true);
  add.focus();
  await element.updateComplete;
  expect(help.hidden).toBe(false);
  add.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await element.updateComplete;
  expect(help.hidden).toBe(true);
  add.blur();
  add.parentElement!.dispatchEvent(new Event("pointerenter"));
  await element.updateComplete;
  expect(help.hidden).toBe(false);
  add.parentElement!.dispatchEvent(new Event("pointerleave"));
  await element.updateComplete;
  expect(help.hidden).toBe(true);
});

it("keeps Add Preset available while Claude is displayed", async () => {
  const { element } = await mount();
  element.selection = { ...element.selection!, candidate: "claude" };
  await element.updateComplete;
  expect(button(element, "Install")).toBeDefined();
  button(element, "Add Preset").click();
  await element.updateComplete;
  expect(button(element, "Install")).toBeUndefined();
  expect(command(element)).toBeDefined();
  expect(button(element, "Reset Presets…")).toBeDefined();
});

it("restores an in-flight Codex update from parent properties after remount", async () => {
  const { element } = await mount();
  element.selection = { ...element.selection!, candidate: "codex" };
  element.runtime = { agent: "codex", stage: "ready", downloaded_bytes: 0 };
  element.updatePending = true;
  element.updateAgent = "codex";
  await element.updateComplete;
  element.remove();
  const replacement = new LensAgentSettings();
  replacement.selection = element.selection;
  replacement.runtime = element.runtime;
  replacement.profiles = element.profiles;
  replacement.updatePending = element.updatePending;
  replacement.updateAgent = element.updateAgent;
  document.body.append(replacement);
  await replacement.updateComplete;
  expect(replacement.querySelector(".runtime-status")?.textContent).toContain(
    "Checking ChatGPT Codex for updates…",
  );
});

it.each(["failed", "unselected"] as const)(
  "shows the external candidate at %s without claiming readiness",
  async (stage) => {
    const { element } = await mount(stage);
    expect(element.querySelector<LensSelect>("lens-select")!.value).toBe("profile-1");
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
  expect(command(element).value).toBe("/chosen path/goose");
  expect(argumentsField(element).value).toBe("acp '' ' literal argument '");
  button(element, "Save and Verify").click();
  expect(intents.at(-1)).toEqual({
    type: "save-external-agent",
    profile: {
      id: "profile-1",
      name: "Goose",
      command: "/chosen path/goose",
      arguments: "acp '' ' literal argument '",
    },
  });
  expect(element.profiles[0]!.command).toBe("goose");
});
it("keeps incomplete arguments local while choosing a new executable", async () => {
  const { element, intents } = await mount();
  await edit(element, "goose 'incomplete");
  expect(element.querySelector('[role="alert"]')).not.toBeNull();
  expect(button(element, "Save and Verify").disabled).toBe(true);
  const revision = browse(element, intents);
  expect(element.acceptExternalExecutable("/chosen/goose", revision)).toBe(true);
  await element.updateComplete;
  expect(command(element).value).toBe("/chosen/goose");
  expect(argumentsField(element).value).toBe("'incomplete");
  expect(button(element, "Save and Verify").disabled).toBe(true);
});
it("retains drafts across managed switches and equivalent snapshots", async () => {
  const { element } = await mount();
  await edit(element, "goose acp --example");
  await choose(element, "claude");
  await choose(element, "profile-1");
  element.profiles = structuredClone(element.profiles);
  await element.updateComplete;
  expect(command(element).value).toBe("goose");
  expect(argumentsField(element).value).toBe("acp --example");
});
it.each(["typing", "arguments", "saved-args", "agent-switch", "new-browse", "disconnect"])(
  "invalidates delayed Browse after %s",
  async (change) => {
    const { element, intents } = await mount();
    const revision = browse(element, intents);
    if (change === "typing") await edit(element, "goose acp --new");
    if (change === "arguments") {
      argumentsField(element).value = 'new "unfinished';
      argumentsField(element).dispatchEvent(new Event("input"));
      await element.updateComplete;
    }
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
  button(element, "Add Preset").click();
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
    [
      ...element
        .querySelector<LensSelect>("lens-select")!
        .shadowRoot!.querySelectorAll('[role="option"]'),
    ].filter((option) => option.textContent?.includes("Goose")),
  ).toHaveLength(2);
  button(element, "Delete Preset").click();
  expect(intents.at(-1)).toEqual({ type: "delete-external-agent", id });
});
it("disables editing while pending and uses advertised logout capability", async () => {
  const { element } = await mount();
  element.disabled = true;
  await element.updateComplete;
  expect(command(element).closest("fieldset")!.disabled).toBe(true);
  expect(button(element, "Reset Presets…").disabled).toBe(true);
  element.disabled = false;
  element.selection = { ...element.selection!, supports_logout: true };
  await element.updateComplete;
  expect(button(element, "Sign Out…")).toBeDefined();
});
it("accepts Browse for an empty new command draft", async () => {
  const { element, intents } = await mount();
  button(element, "Add Preset").click();
  await element.updateComplete;
  const revision = browse(element, intents);
  expect(element.acceptExternalExecutable("/new path/agent", revision)).toBe(true);
  await element.updateComplete;
  expect(command(element).value).toBe("/new path/agent");
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
    {
      id: "a14d73cb-951c-48ed-a305-3829750c88da",
      name: "GitHub Copilot",
      command: "copilot",
      args: ["--acp", "--stdio"],
    },
    { id: "6b57315e-9c13-4e4a-bf4c-e6bc33b10b21", name: "Goose", command: "goose", args: ["acp"] },
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
  expect(element.querySelectorAll("lens-select")).toHaveLength(1);
  const selector = element.querySelector<LensSelect>("lens-select")!;
  expect(selector.options.map((option) => option.label)).toEqual([
    "Claude Code",
    "ChatGPT Codex",
    "GitHub Copilot",
    "Goose",
  ]);
  expect(intents).toEqual([]);
  await choose(element, presets[0]!.id);
  expect(selector.value).toBe(presets[0]!.id);
  expect(command(element).value).toBe("copilot");
  expect(argumentsField(element).value).toBe("--acp --stdio");
  expect(intents).toEqual([{ type: "select", agent: { external: presets[0]!.id } }]);
  expect(button(element, "Delete Preset")).toBeDefined();
});

it("resets saved edits and unsaved drafts only after acceptance and invalidates pending Browse", async () => {
  const { element, intents } = await mount();
  await edit(element, "goose acp --edited");
  button(element, "Add Preset").click();
  await element.updateComplete;
  await edit(element, "custom-agent");
  const revision = browse(element, intents);
  button(element, "Reset Presets…").click();
  expect(intents.at(-1)).toEqual({ type: "reset-agent-presets" });
  expect(command(element).value).toBe("custom-agent");
  const count = intents.length;
  element.acceptResetPresets(element.profiles, "claude");
  await element.updateComplete;
  expect(element.acceptExternalExecutable("/late/agent", revision)).toBe(false);
  expect(element.querySelector<LensSelect>("lens-select")!.value).toBe("claude");
  expect(
    element
      .querySelector<LensSelect>("lens-select")!
      .shadowRoot!.querySelectorAll('[role="option"]'),
  ).toHaveLength(3);
  expect(intents).toHaveLength(count);
  await choose(element, "profile-1");
  expect(command(element).value).toBe("goose");
});

it.each(["unselected", "failed", "authentication_required"] as const)(
  "can explicitly verify the current managed choice at %s without selecting another agent",
  async (stage) => {
    const { element, intents } = await mount(stage);
    element.selection = { ...element.selection!, candidate: "claude" };
    await element.updateComplete;
    expect(intents).toEqual([]);
    button(element, "Verify Connection").click();
    expect(intents).toEqual([{ type: "select", agent: "claude" }]);
  },
);
it("returns to the authoritative external choice after discarding a new draft without launching", async () => {
  const { element, intents } = await mount();
  await edit(element, "goose acp --saved-profile-draft");
  button(element, "Add Preset").click();
  await element.updateComplete;
  await edit(element, "another-agent");
  button(element, "Discard Draft").click();
  await element.updateComplete;
  expect(element.querySelector<LensSelect>("lens-select")!.value).toBe("profile-1");
  expect(command(element).value).toBe("goose");
  expect(argumentsField(element).value).toBe("acp --saved-profile-draft");
  expect(intents).toEqual([]);
});

it("submits the external draft once while preserving validation and separate browse actions", async () => {
  const { element, intents } = await mount();
  const form = element.querySelector("form")!;
  const save = button(element, "Save and Verify");
  expect(save.type).toBe("submit");
  expect(save.dataset.lensButtonRole).toBe("primary");
  form.requestSubmit(save);
  expect(intents).toHaveLength(1);
  expect(intents[0]).toMatchObject({
    type: "save-external-agent",
    profile: { id: "profile-1", name: "Goose", command: "goose", arguments: "acp" },
  });
  intents.length = 0;
  button(element, "Choose…").click();
  expect(intents).toHaveLength(1);
  expect(intents[0]?.type).toBe("choose-external-executable");
  intents.length = 0;
  await edit(element, "");
  form.requestSubmit(save);
  expect(intents).toEqual([]);
  await edit(element, "goose acp");
  element.disabled = true;
  await element.updateComplete;
  form.requestSubmit(save);
  expect(intents).toEqual([]);
});
