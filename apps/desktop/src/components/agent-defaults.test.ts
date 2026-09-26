import type { LensSelect } from "./lens-select";
// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import type { AgentDefaults, AgentSelectionState } from "../types";
import { DEFAULT_AGENT_DEFAULTS, LensAgentDefaults } from "./lens-agent-defaults";
import type { AgentIntent } from "./events";

afterEach(() => document.body.replaceChildren());

function selection(): AgentSelectionState {
  return {
    stage: "selected",
    candidate: "codex",
    supports_logout: false,
    auth_methods: [],
    config_options: [
      {
        id: "model",
        name: "Model",
        type: "select",
        category: "model",
        currentValue: "second",
        options: [
          {
            group: "models",
            name: "Models",
            options: [
              { value: "first", name: "First" },
              { value: "second", name: "Second" },
            ],
          },
        ],
      },
      {
        id: "effort",
        name: "Reasoning effort",
        type: "select",
        category: "thought_level",
        currentValue: "medium",
        options: [
          { value: "medium", name: "Medium" },
          { value: "high", name: "High" },
        ],
      },
      {
        id: "mode",
        name: "Mode",
        type: "select",
        category: "mode",
        currentValue: "safe",
        options: [
          { value: "safe", name: "Safe" },
          { value: "write", name: "Write" },
        ],
      },
    ],
  };
}
function saved(): AgentDefaults {
  return {
    ...structuredClone(DEFAULT_AGENT_DEFAULTS),
    choices: [
      { config_id: "model", value: "second" },
      { config_id: "effort", value: "high" },
      { config_id: "mode", value: "write" },
    ],
  };
}
async function mount(defaults = saved(), state = selection()) {
  const element = document.createElement("lens-agent-defaults") as LensAgentDefaults;
  element.defaults = defaults;
  element.selection = state;
  const intents: AgentIntent[] = [];
  element.addEventListener("lens-agent-intent", (event) =>
    intents.push((event as CustomEvent<AgentIntent>).detail),
  );
  document.body.append(element);
  await element.updateComplete;
  return { element, intents };
}
function select(element: LensAgentDefaults, label: string) {
  return element.querySelector<LensSelect>(`lens-select[label="${label} default"]`)!;
}
function click(element: LensAgentDefaults, label: string) {
  const button = [...element.querySelectorAll("button")].find(
    (item) => item.textContent?.trim() === label,
  );
  expect(button).toBeTruthy();
  button!.click();
}
async function choose(element: LensAgentDefaults, label: string, value: string) {
  const control = select(element, label);
  await control.updateComplete;
  control.shadowRoot!.querySelector<HTMLButtonElement>("button")!.click();
  await control.updateComplete;
  control
    .shadowRoot!.querySelector<HTMLElement>(
      `[data-index="${control.options.findIndex((option) => option.value === value)}"]`,
    )!
    .click();
  await element.updateComplete;
}
function expectSavedDisplay(element: LensAgentDefaults) {
  expect(select(element, "Model").value).toBe("second");
  expect(select(element, "Reasoning effort").value).toBe("high");
  expect(select(element, "Mode").value).toBe("write");
}

describe("Model & Behavior selection persistence", () => {
  it("shows an external agent's current provider as externally managed and excludes stored overrides from Save and Revert", async () => {
    const state = selection();
    state.candidate = { external: "profile-1" };
    state.config_options!.push({
      id: "provider",
      name: "Provider",
      type: "select",
      currentValue: "current-provider",
      options: [
        { value: "current-provider", name: "Configured provider" },
        { value: "stale-provider", name: "Stale provider" },
      ],
    });
    const defaults = saved();
    defaults.choices.push({ config_id: "provider", value: "stale-provider" });
    const { element, intents } = await mount(defaults, state);
    expect(element.querySelector('lens-select[data-agent-config-id="provider"]')).toBeNull();
    expect(
      element.querySelector('output[aria-label="Provider managed by external CLI"]')?.textContent,
    ).toBe("Configured provider");
    expect(element.textContent).toContain("agent's own CLI");
    expect(element.textContent?.replace(/\s+/g, " ")).toContain("Save and Verify in Connection");
    expectSavedDisplay(element);
    click(element, "Save Defaults");
    expect(intents.at(-1)).toEqual({ type: "save-defaults", defaults: saved() });
    click(element, "Revert");
    await element.updateComplete;
    click(element, "Save Defaults");
    expect(intents.at(-1)).toEqual({ type: "save-defaults", defaults: saved() });
    expect(defaults.choices.at(-1)).toEqual({ config_id: "provider", value: "stale-provider" });
  });

  it("keeps advertised provider options editable for other Agents", async () => {
    const state = selection();
    state.config_options!.push({
      id: "provider",
      name: "Provider",
      type: "select",
      currentValue: "first",
      options: [
        { value: "first", name: "First" },
        { value: "second", name: "Second" },
      ],
    });
    const { element, intents } = await mount(saved(), state);
    await choose(element, "Provider", "second");
    click(element, "Save Defaults");
    expect(intents.at(-1)).toEqual({
      type: "save-defaults",
      defaults: {
        ...saved(),
        choices: [...saved().choices, { config_id: "provider", value: "second" }],
      },
    });
  });
  it("uses Agent default for unset options and Ask for Other requests", async () => {
    const { element, intents } = await mount(structuredClone(DEFAULT_AGENT_DEFAULTS));
    for (const label of ["Model", "Reasoning effort", "Mode"]) {
      expect(select(element, label).value).toBe("");
      expect(
        select(element, label).options.find(
          (option) => option.value === select(element, label).value,
        )?.label,
      ).toBe("Agent default");
    }
    const other = element.querySelector<LensSelect>('lens-select[label="Other requests policy"]')!;
    expect(other.value).toBe("ask");
    expect(other.closest(".settings-field")?.textContent).toContain(
      "Requests without a recognized classification",
    );
    expect(other.closest(".settings-field")?.textContent).toContain(
      "including HTML output publication",
    );
    other.value = "deny";
    other.dispatchEvent(new Event("change"));
    await element.updateComplete;
    click(element, "Save Defaults");
    expect(intents.at(-1)).toMatchObject({
      type: "save-defaults",
      defaults: { choices: [], tools: { other: "deny" } },
    });
    click(element, "Revert");
    await element.updateComplete;
    expect(other.value).toBe("ask");
  });
  it("keeps all eight editable policies collapsed while Save and Revert remain outside", async () => {
    const { element, intents } = await mount();
    const details = element.querySelector<HTMLDetailsElement>("details")!;
    expect(details.open).toBe(false);
    const policies = details.querySelectorAll<LensSelect>('lens-select[label$=" policy"]');
    expect(policies).toHaveLength(8);
    for (const policy of policies) {
      expect([...policy.options].map((option) => option.value)).toEqual(["ask", "allow", "deny"]);
      policy.value = "allow";
      policy.dispatchEvent(new Event("change", { bubbles: true }));
    }
    await element.updateComplete;
    const save = [...element.querySelectorAll("button")].find((button) =>
      button.textContent?.includes("Save Defaults"),
    )!;
    expect(save.closest("details")).toBeNull();
    save.click();
    expect(intents.at(-1)).toMatchObject({
      type: "save-defaults",
      defaults: {
        tools: {
          read: "allow",
          search: "allow",
          fetch: "allow",
          edit: "allow",
          delete: "allow",
          move: "allow",
          execute: "allow",
          other: "allow",
        },
      },
    });
  });
  it("displays saved flat and grouped choices on first mount and reopening without emitting edits", async () => {
    for (let opening = 0; opening < 2; opening++) {
      const { element, intents } = await mount();
      expectSavedDisplay(element);
      expect(intents).toEqual([]);
      click(element, "Save Defaults");
      expect(intents).toEqual([{ type: "save-defaults", defaults: saved() }]);
      element.remove();
    }
  });

  it("preserves selection through catalog reorder, regrouping, removal and return", async () => {
    const { element, intents } = await mount();
    const state = selection();
    state.config_options = state.config_options!.reverse().map((option) => ({
      ...option,
      options: [
        {
          group: "replacement",
          name: "Replacement",
          options: [
            { value: "write", name: "Write" },
            { value: "high", name: "High" },
            { value: "second", name: "Second" },
          ],
        },
      ],
    }));
    element.selection = state;
    await element.updateComplete;
    expectSavedDisplay(element);
    element.selection = {
      ...state,
      config_options: state.config_options.map((option) => ({ ...option, options: [] })),
    };
    await element.updateComplete;
    expectSavedDisplay(element);
    for (const label of ["Model", "Reasoning effort", "Mode"]) {
      const option = select(element, label).options.find(
        (option) => option.value === select(element, label).value,
      )!;
      expect(option.disabled).toBe(true);
      expect(option.label).toContain("not in current choices");
    }
    expect(intents).toEqual([]);
    click(element, "Save Defaults");
    expect(intents).toEqual([{ type: "save-defaults", defaults: saved() }]);
    element.selection = selection();
    await element.updateComplete;
    expectSavedDisplay(element);
    expect(element.textContent).not.toContain("not in current choices");
  });

  it("keeps edits across equal snapshots and restores saved choices on Revert", async () => {
    const { element, intents } = await mount();
    await choose(element, "Mode", "safe");
    await choose(element, "Reasoning effort", "medium");
    element.defaults = saved();
    element.selection = selection();
    await element.updateComplete;
    expect(select(element, "Mode").value).toBe("safe");
    expect(select(element, "Reasoning effort").value).toBe("medium");
    click(element, "Save Defaults");
    expect(intents.at(-1)).toMatchObject({
      type: "save-defaults",
      defaults: {
        choices: [
          { config_id: "model", value: "second" },
          { config_id: "mode", value: "safe" },
          { config_id: "effort", value: "medium" },
        ],
      },
    });
    click(element, "Revert");
    await element.updateComplete;
    expectSavedDisplay(element);
    expect(intents.at(-1)).toEqual({ type: "preview-model", configId: "model", value: "second" });
  });

  it("replaces an updated runtime catalog and resets only invalid draft choices", async () => {
    const state = { ...selection(), catalog_generation: "old", catalog_revision: 1 };
    const { element, intents } = await mount(saved(), state);
    await choose(element, "Mode", "safe");
    const policy = element.querySelector<LensSelect>(
      'lens-select[label="Read files or data policy"]',
    )!;
    policy.value = "allow";
    policy.dispatchEvent(new Event("change"));
    const next = { ...selection(), catalog_generation: "new", catalog_revision: 2 };
    next.config_options = next.config_options!.map((option) =>
      option.category === "model"
        ? { ...option, options: [{ value: "third", name: "Third" }], currentValue: "third" }
        : option,
    );
    element.defaults = {
      ...saved(),
      choices: saved().choices.filter((choice) => choice.config_id !== "model"),
    };
    element.selection = next;
    await element.updateComplete;
    expect(select(element, "Model").options.some((option) => option.value === "third")).toBe(true);
    expect(select(element, "Model").options.some((option) => option.value === "second")).toBe(
      false,
    );
    expect(select(element, "Model").value).toBe("");
    expect(select(element, "Mode").value).toBe("safe");
    expect(policy.value).toBe("allow");
    expect(element.textContent).toContain("These choices now use Agent default");
    expect(intents).toEqual([]);
    click(element, "Save Defaults");
    expect(intents.at(-1)).toMatchObject({
      type: "save-defaults",
      defaults: {
        choices: [
          { config_id: "effort", value: "high" },
          { config_id: "mode", value: "safe" },
        ],
        tools: { read: "allow" },
      },
    });
  });

  it("refreshes an unsaved model after update before reconciling its dependent choices", async () => {
    const state = { ...selection(), catalog_generation: "old", catalog_revision: 1 };
    const { element, intents } = await mount(saved(), state);
    await choose(element, "Model", "first");
    await choose(element, "Reasoning effort", "high");
    const next = { ...selection(), catalog_generation: "new", catalog_revision: 2 };
    next.config_options = next.config_options!.map((option) =>
      option.category === "thought_level"
        ? { ...option, options: [{ value: "medium", name: "Medium" }] }
        : option,
    );
    element.disabled = true;
    element.selection = next;
    await element.updateComplete;
    expect(intents).toHaveLength(1);
    expect(select(element, "Model").value).toBe("first");
    expect(select(element, "Reasoning effort").value).toBe("high");
    element.disabled = false;
    await element.updateComplete;
    expect(intents).toHaveLength(2);
    expect(intents.at(-1)).toEqual({ type: "preview-model", configId: "model", value: "first" });
    expect(select(element, "Reasoning effort").disabled).toBe(true);
    click(element, "Save Defaults");
    expect(intents).toHaveLength(2);
    element.selection = structuredClone(next);
    await element.updateComplete;
    expect(intents).toHaveLength(2);
    expect(select(element, "Reasoning effort").disabled).toBe(true);
    element.selection = {
      ...next,
      catalog_revision: 3,
      catalog_model: "first",
      config_options: selection().config_options!.map((option) =>
        option.category === "model" ? { ...option, currentValue: "first" } : option,
      ),
    };
    await element.updateComplete;
    expect(select(element, "Reasoning effort").disabled).toBe(false);
    expect(select(element, "Reasoning effort").value).toBe("high");
    click(element, "Save Defaults");
    expect(intents.at(-1)).toMatchObject({
      type: "save-defaults",
      defaults: {
        choices: [
          { config_id: "mode", value: "write" },
          { config_id: "model", value: "first" },
          { config_id: "effort", value: "high" },
        ],
      },
    });
  });

  it("resolves a removed unsaved model against Agent default instead of the saved model catalog", async () => {
    const { element, intents } = await mount(saved(), {
      ...selection(),
      catalog_generation: "old",
      catalog_revision: 1,
    });
    await choose(element, "Model", "first");
    await choose(element, "Reasoning effort", "high");
    element.selection = {
      ...selection(),
      catalog_generation: "new",
      catalog_revision: 2,
      catalog_model: "second",
      config_options: selection().config_options!.map((option) =>
        option.category === "model"
          ? { ...option, options: [{ value: "second", name: "Second" }] }
          : option,
      ),
    };
    await element.updateComplete;
    expect(select(element, "Model").value).toBe("");
    expect(select(element, "Reasoning effort").value).toBe("high");
    expect(intents.at(-1)).toEqual({ type: "preview-model", configId: "model", value: undefined });
    element.selection = { ...element.selection!, catalog_revision: 3, catalog_model: null };
    await element.updateComplete;
    expect(select(element, "Reasoning effort").disabled).toBe(false);
    expect(select(element, "Reasoning effort").value).toBe("high");
  });

  it("does not accept a different model preview as completion after a newer update", async () => {
    const { element, intents } = await mount(saved(), {
      ...selection(),
      catalog_generation: "old",
      catalog_revision: 1,
    });
    await choose(element, "Model", "first");
    element.selection = {
      ...selection(),
      catalog_generation: "new",
      catalog_revision: 2,
      catalog_model: "second",
    };
    await element.updateComplete;
    element.selection = {
      ...selection(),
      catalog_generation: "newer",
      catalog_revision: 3,
      catalog_model: "second",
    };
    await element.updateComplete;
    expect(intents).toHaveLength(3);
    element.selection = { ...element.selection!, catalog_revision: 4 };
    await element.updateComplete;
    expect(select(element, "Reasoning effort").disabled).toBe(true);
    click(element, "Save Defaults");
    expect(intents).toHaveLength(3);
    click(element, "Refresh Model Settings");
    element.selection = { ...element.selection!, catalog_revision: 5, catalog_model: "first" };
    await element.updateComplete;
    expect(select(element, "Reasoning effort").disabled).toBe(false);
    expect(intents).toHaveLength(4);
  });

  it("removes missing configuration IDs on update while retaining renamed choices and tools", async () => {
    const { element, intents } = await mount(saved(), {
      ...selection(),
      catalog_generation: "old",
      catalog_revision: 1,
    });
    const next = { ...selection(), catalog_generation: "new", catalog_revision: 2 };
    next.config_options = next
      .config_options!.filter((option) => option.category !== "model")
      .map((option) => (option.category === "mode" ? { ...option, name: "Updated mode" } : option));
    element.selection = next;
    await element.updateComplete;
    expect(select(element, "Model")).toBeNull();
    expect(select(element, "Updated mode").value).toBe("write");
    expect(intents).toEqual([]);
    click(element, "Save Defaults");
    expect(intents.at(-1)).toMatchObject({
      type: "save-defaults",
      defaults: {
        choices: [
          { config_id: "effort", value: "high" },
          { config_id: "mode", value: "write" },
        ],
        tools: saved().tools,
      },
    });
  });

  it("resolves an unsaved Agent default model and permits explicit refresh after failure", async () => {
    const { element, intents } = await mount(saved(), {
      ...selection(),
      catalog_generation: "old",
      catalog_revision: 1,
    });
    await choose(element, "Model", "");
    await choose(element, "Reasoning effort", "high");
    element.selection = { ...selection(), catalog_generation: "new", catalog_revision: 2 };
    await element.updateComplete;
    expect(intents.at(-1)).toEqual({ type: "preview-model", configId: "model", value: undefined });
    expect(select(element, "Reasoning effort").disabled).toBe(true);
    click(element, "Refresh Model Settings");
    expect(intents).toHaveLength(3);
    element.selection = {
      ...selection(),
      catalog_generation: "new",
      catalog_revision: 3,
      config_options: selection().config_options!.map((option) =>
        option.category === "thought_level"
          ? { ...option, options: [{ value: "medium", name: "Medium" }] }
          : option,
      ),
    };
    await element.updateComplete;
    expect(select(element, "Reasoning effort").value).toBe("");
    expect(select(element, "Reasoning effort").disabled).toBe(false);
    expect(element.textContent).toContain("Reasoning effort. These choices now use Agent default");
  });

  it("clears reasoning when the user switches model and applies changed saved defaults", async () => {
    const { element, intents } = await mount();
    await choose(element, "Model", "first");
    expect(select(element, "Model").value).toBe("first");
    expect(select(element, "Reasoning effort").value).toBe("");
    expect(intents).toEqual([{ type: "preview-model", configId: "model", value: "first" }]);
    click(element, "Save Defaults");
    expect(intents.at(-1)).toMatchObject({
      type: "save-defaults",
      defaults: {
        choices: [
          { config_id: "mode", value: "write" },
          { config_id: "model", value: "first" },
        ],
      },
    });
    element.defaults = { ...saved(), choices: [{ config_id: "mode", value: "safe" }] };
    await element.updateComplete;
    expect(select(element, "Model").value).toBe("");
    expect(select(element, "Reasoning effort").value).toBe("");
    expect(select(element, "Mode").value).toBe("safe");
  });

  it("restores legacy fallback modes and preserves an unavailable saved mode", async () => {
    const defaults = { ...saved(), choices: [{ config_id: "mode", value: "write" }] };
    const state = {
      ...selection(),
      config_options: undefined,
      modes: [
        { id: "safe", name: "Safe" },
        { id: "write", name: "Write" },
      ],
    };
    const { element, intents } = await mount(defaults, state);
    expect(select(element, "Mode").value).toBe("write");
    element.selection = { ...state, modes: [] };
    await element.updateComplete;
    expect(select(element, "Mode").value).toBe("write");
    expect(
      select(element, "Mode").options.find(
        (option) => option.value === select(element, "Mode").value,
      )?.disabled,
    ).toBe(true);
    expect(intents).toEqual([]);
    click(element, "Save Defaults");
    expect(intents).toEqual([{ type: "save-defaults", defaults }]);
    await choose(element, "Mode", "");
    expect(
      select(element, "Mode").options.find(
        (option) => option.value === select(element, "Mode").value,
      )?.label,
    ).toContain("Agent default");
    click(element, "Save Defaults");
    expect(intents.at(-1)).toMatchObject({ type: "save-defaults", defaults: { choices: [] } });
  });
});

it("uses the shared control for every default and explicitly disables custom triggers", async () => {
  const { element, intents } = await mount();
  expect(element.querySelector("select")).toBeNull();
  const controls = [...element.querySelectorAll<LensSelect>("lens-select")];
  expect(controls.length).toBeGreaterThan(8);
  element.disabled = true;
  await element.updateComplete;
  await Promise.all(controls.map((control) => control.updateComplete));
  expect(
    controls.every(
      (control) =>
        control.disabled &&
        control.shadowRoot!.querySelector<HTMLButtonElement>("button")!.disabled,
    ),
  ).toBe(true);
  expect(intents).toEqual([]);
});
