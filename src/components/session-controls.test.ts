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
  it("renders Agent order and sends exact IDs with session revision", async () => {
    const element = await mount(snapshot());
    const intents: OverlayIntent[] = [];
    element.addEventListener("lens-overlay-intent", (e) =>
      intents.push((e as CustomEvent<OverlayIntent>).detail),
    );
    const select = element.querySelector("select")!;
    expect([...select.options].map((o) => o.value)).toEqual(["second", "first"]);
    expect(select.value).toBe("second");
    select.value = "first";
    select.dispatchEvent(new Event("change"));
    expect(intents).toEqual([
      {
        type: "set-session-option",
        instanceId: "instance",
        revision: 4,
        configId: "model",
        value: "first",
      },
    ]);
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
    expect(element.querySelector("select")?.disabled).toBe(true);
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
    expect(element.querySelector("select")?.disabled).toBe(true);
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
    const policy = element.querySelector<HTMLSelectElement>(
      'select[aria-label="Read files or data policy"]',
    )!;
    policy.value = "deny";
    policy.dispatchEvent(new Event("change"));
    await element.updateComplete;
    expect(intents).toEqual([]);
    click(element, "Save Shared Settings");
    expect(intents[0]).toMatchObject({
      type: "save-defaults",
      defaults: {
        choices: [{ config_id: "model", value: "first" }],
        tools: { read: "deny", execute: "deny" },
      },
    });
  });
});
