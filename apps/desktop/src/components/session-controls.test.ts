// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { AgentSessionControlState } from "../types";
import type { LensSessionControls } from "./lens-session-controls";
import type { LensAgentDefaults } from "./lens-agent-defaults";
import type { OverlayIntent, AgentIntent } from "./events";

beforeAll(async () => {
  await import("./lens-session-controls");
  await import("./lens-agent-defaults");
});
afterEach(() => document.body.replaceChildren());
function snapshot(): AgentSessionControlState {
  return {
    instance_id: "instance",
    operation_id: "operation",
    session_id: "session",
    agent_name: "Synthetic Agent",
    active: true,
    config_revision: 4,
    policy_default: "safe",
    effective_mode: "safe",
    modes: [],
    config_options: [
      {
        id: "model",
        name: "Agent Model",
        type: "select",
        category: "model",
        currentValue: "second",
        options: [
          {
            group: "z",
            name: "Agent Group",
            options: [
              { value: "second", name: "Second" },
              { value: "first", name: "First" },
            ],
          },
        ],
      },
    ],
    interactions: [],
  };
}
async function mount(controls: AgentSessionControlState) {
  const element = document.createElement("lens-session-controls") as LensSessionControls;
  element.controls = controls;
  element.presentation = controls.interactions.some((i) => i.status === "pending")
    ? "interaction"
    : "diagnostics";
  document.body.append(element);
  await element.updateComplete;
  return element;
}
function click(element: HTMLElement, label: string) {
  const button = [...element.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === label,
  );
  expect(button).toBeTruthy();
  button!.click();
}
describe("session control boundary", () => {
  it("shows diagnostics without session setting controls", async () => {
    const element = await mount(snapshot());
    expect(element.querySelector("select")).toBeNull();
    expect(element.textContent).not.toContain("Session settings");
    expect(element.textContent).toContain("Synthetic Agent");
  });
  it("renders permission arguments as escaped text and only sends an advertised option ID", async () => {
    const controls = snapshot();
    controls.interactions = [
      {
        id: "decision",
        sequence: 1,
        status: "pending",
        details: {
          kind: "permission",
          tool_call_id: "tool",
          title: "Read fixture",
          effect: "read",
          arguments: { path: "<img src=x onerror=alert(1)>" },
          options: [{ optionId: "exact-allow-id", name: "Allow Once", kind: "allow_once" }],
        },
      },
    ];
    const element = await mount(controls);
    const intents: OverlayIntent[] = [];
    element.addEventListener("lens-overlay-intent", (e) =>
      intents.push((e as CustomEvent<OverlayIntent>).detail),
    );
    expect(element.querySelector("img")).toBeNull();
    expect(element.querySelector("pre")?.textContent).toContain("<img");
    expect(element.querySelector("select")).toBeNull();
    click(element, "Allow Once");
    expect(intents).toEqual([
      {
        type: "respond-interaction",
        instanceId: "instance",
        interactionId: "decision",
        response: { action: "select", option_id: "exact-allow-id" },
      },
    ]);
  });
  it("keeps the oldest request and form draft stable during updates and retries", async () => {
    const controls = snapshot();
    const first = {
      id: "first",
      sequence: 1,
      status: "pending" as const,
      details: {
        kind: "form" as const,
        message: "First request",
        schema: {
          type: "object" as const,
          properties: { answer: { type: "string" as const } },
          required: ["answer"],
        },
      },
    };
    controls.interactions = [
      {
        ...first,
        id: "second",
        sequence: 2,
        details: { ...first.details, message: "Second request" },
      },
      first,
    ];
    const element = await mount(controls);
    expect(element.querySelectorAll('button[type="submit"]')).toHaveLength(1);
    expect(element.querySelector(".interaction-actions button[type=submit]")).not.toBeNull();
    const input = element.querySelector<HTMLInputElement>("input")!;
    input.value = "Keep my draft";
    element.controls = { ...controls, notice: "Updated" };
    await element.updateComplete;
    expect(element.querySelector("input")).toBe(input);
    expect(input.value).toBe("Keep my draft");
    expect(element.textContent).not.toContain("Second request");
    element.submission = { instanceId: "instance", interactionId: "first", stage: "sending" };
    await element.updateComplete;
    expect(element.querySelector("fieldset")?.disabled).toBe(true);
    element.submission = { ...element.submission, stage: "failed", message: "Transport failed" };
    await element.updateComplete;
    expect(element.querySelector("fieldset")?.disabled).toBe(false);
    expect(input.value).toBe("Keep my draft");
    expect(element.querySelector('[role="alert"]')?.textContent).toContain("Transport failed");
    element.controls = {
      ...controls,
      interactions: controls.interactions.map((i) =>
        i.id === "first" ? { ...i, status: "accepted", details: undefined } : i,
      ),
    };
    await element.updateComplete;
    expect(element.textContent).toContain("Second request");
    expect(element.querySelector<HTMLInputElement>("input")?.value).toBe("");
    element.controls = { ...controls, active: false };
    await element.updateComplete;
    expect(element.querySelector("button")).toBeNull();
  });

  it("requires a separate explicit mode confirmation", async () => {
    const controls = snapshot();
    controls.interactions = [
      {
        id: "mode",
        sequence: 1,
        status: "pending",
        details: { kind: "mode_transition", from: "safe", to: "write" },
      },
    ];
    const element = await mount(controls);
    const intents: OverlayIntent[] = [];
    element.addEventListener("lens-overlay-intent", (e) =>
      intents.push((e as CustomEvent<OverlayIntent>).detail),
    );
    expect(element.textContent).toContain("safe → write");
    expect(intents).toEqual([]);
    click(element, "Confirm mode change");
    expect(intents[0]).toMatchObject({ response: { action: "accept" } });
  });
  it("collects typed form values without opening a URL implicitly", async () => {
    const controls = snapshot();
    controls.interactions = [
      {
        id: "form",
        sequence: 1,
        status: "pending",
        details: {
          kind: "form",
          message: "Choose a count",
          schema: {
            type: "object",
            properties: {
              count: { type: "integer", minimum: 1, maximum: 3 },
              enabled: { type: "boolean" },
            },
            required: ["count"],
          },
        },
      },
    ];
    const element = await mount(controls);
    const intents: OverlayIntent[] = [];
    element.addEventListener("lens-overlay-intent", (e) =>
      intents.push((e as CustomEvent<OverlayIntent>).detail),
    );
    const count = element.querySelector<HTMLInputElement>('input[name="count"]')!;
    count.value = "2";
    element.querySelector<HTMLSelectElement>('select[name="enabled"]')!.value = "false";
    element
      .querySelector("form")!
      .dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));
    expect(intents).toEqual([
      {
        type: "respond-interaction",
        instanceId: "instance",
        interactionId: "form",
        response: { action: "submit", content: { count: 2, enabled: false } },
      },
    ]);
  });
  it("displays the complete URL and emits consent only on activation", async () => {
    const controls = snapshot();
    controls.interactions = [
      {
        id: "url",
        sequence: 1,
        status: "pending",
        details: {
          kind: "url",
          elicitation_id: "url-id",
          message: "Connect",
          url: "https://example.com/authorize?state=fixture",
        },
      },
    ];
    const element = await mount(controls);
    const intents: OverlayIntent[] = [];
    element.addEventListener("lens-overlay-intent", (e) =>
      intents.push((e as CustomEvent<OverlayIntent>).detail),
    );
    expect(element.textContent).toContain("https://example.com/authorize?state=fixture");
    expect(element.querySelector("a")).toBeNull();
    expect(intents).toEqual([]);
    click(element, "Open URL and continue");
    expect(intents[0]).toMatchObject({ response: { action: "accept" } });
  });
  it("disables ended-session settings and removes terminal forms", async () => {
    const controls = snapshot();
    controls.active = false;
    controls.interactions = [{ id: "done", sequence: 1, status: "expired" }];
    const element = await mount(controls);
    expect(element.querySelector("select")).toBeNull();
    expect(element.querySelector("form")).toBeNull();
    expect(element.textContent).toContain("expired");
  });
});
describe("shared Agent defaults", () => {
  it("keeps common policies and exact saved choices separate from session state", async () => {
    const element = document.createElement("lens-agent-defaults") as LensAgentDefaults;
    element.selection = {
      stage: "selected",
      candidate: "codex",
      auth_methods: [],
      config_options: snapshot().config_options,
      policy_default: "safe",
    };
    document.body.append(element);
    await element.updateComplete;
    const intents: AgentIntent[] = [];
    element.addEventListener("lens-agent-intent", (event) =>
      intents.push((event as CustomEvent<AgentIntent>).detail),
    );
    const model = element.querySelector<HTMLSelectElement>(
      'select[aria-label="Agent Model default"]',
    )!;
    expect(model.value).toBe("");
    model.value = "first";
    model.dispatchEvent(new Event("change"));
    await element.updateComplete;
    expect(
      element.querySelector<HTMLSelectElement>('select[aria-label="Reasoning effort default"]')
        ?.disabled,
    ).toBe(true);
    element.selection = {
      ...element.selection,
      config_options: [
        ...snapshot().config_options!,
        {
          id: "reasoning_effort",
          name: "Reasoning effort",
          category: "thought_level",
          type: "select",
          currentValue: "medium",
          options: [
            { value: "medium", name: "Medium" },
            { value: "high", name: "High" },
          ],
        },
      ],
    };
    await element.updateComplete;
    const reasoning = element.querySelector<HTMLSelectElement>(
      'select[aria-label="Reasoning effort default"]',
    )!;
    expect(reasoning.disabled).toBe(false);
    reasoning.value = "high";
    reasoning.dispatchEvent(new Event("change"));
    await element.updateComplete;
    const policy = element.querySelector<HTMLSelectElement>(
      'select[aria-label="Read files or data policy"]',
    )!;
    expect(element.textContent).toContain("Permission request response policy");
    expect(element.textContent).toContain("Operations without a request follow the Agent");
    expect(Array.from(policy.options).map((option) => option.value)).toEqual([
      "ask",
      "allow",
      "deny",
    ]);
    policy.value = "allow";
    policy.dispatchEvent(new Event("change"));
    await element.updateComplete;
    expect(intents).toEqual([{ type: "preview-model", configId: "model", value: "first" }]);
    const { DEFAULT_AGENT_DEFAULTS } = await import("./lens-agent-defaults");
    element.defaults = structuredClone(DEFAULT_AGENT_DEFAULTS);
    element.selection = structuredClone(element.selection);
    await element.updateComplete;
    expect(model.value).toBe("first");
    expect(policy.value).toBe("allow");
    click(element, "Save Shared Settings");
    expect(intents[1]).toMatchObject({
      type: "save-defaults",
      defaults: {
        choices: [
          { config_id: "model", value: "first" },
          { config_id: "reasoning_effort", value: "high" },
        ],
        tools: { read: "allow", execute: "deny" },
      },
    });
  });
});
