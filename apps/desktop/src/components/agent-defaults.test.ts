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
  return element.querySelector<HTMLSelectElement>(`select[aria-label="${label} default"]`)!;
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
  control.value = value;
  control.dispatchEvent(new Event("change"));
  await element.updateComplete;
}
function expectSavedDisplay(element: LensAgentDefaults) {
  expect(select(element, "Model").value).toBe("second");
  expect(select(element, "Reasoning effort").value).toBe("high");
  expect(select(element, "Mode").value).toBe("write");
}

describe("Shared Agent Settings selection persistence", () => {
  it("displays saved flat and grouped choices on first mount and reopening without emitting edits", async () => {
    for (let opening = 0; opening < 2; opening++) {
      const { element, intents } = await mount();
      expectSavedDisplay(element);
      expect(intents).toEqual([]);
      click(element, "Save Shared Settings");
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
      const option = select(element, label).selectedOptions[0]!;
      expect(option.disabled).toBe(true);
      expect(option.textContent).toContain("not in current choices");
    }
    expect(intents).toEqual([]);
    click(element, "Save Shared Settings");
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
    click(element, "Save Shared Settings");
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

  it("clears reasoning when the user switches model and applies changed saved defaults", async () => {
    const { element, intents } = await mount();
    await choose(element, "Model", "first");
    expect(select(element, "Model").value).toBe("first");
    expect(select(element, "Reasoning effort").value).toBe("");
    expect(intents).toEqual([{ type: "preview-model", configId: "model", value: "first" }]);
    click(element, "Save Shared Settings");
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
    expect(select(element, "Mode").selectedOptions[0]?.disabled).toBe(true);
    expect(intents).toEqual([]);
    click(element, "Save Shared Settings");
    expect(intents).toEqual([{ type: "save-defaults", defaults }]);
    await choose(element, "Mode", "");
    expect(select(element, "Mode").selectedOptions[0]?.textContent).toContain("Lens safe default");
    click(element, "Save Shared Settings");
    expect(intents.at(-1)).toMatchObject({ type: "save-defaults", defaults: { choices: [] } });
  });
});
