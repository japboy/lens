// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { OverlayViewModel, TargetSelectionViewModel } from "../application/view-models";
import type { LensTargetSelectionItem } from "../types";
import {
  PROMPT_INTENT_EVENT,
  TARGET_SELECTION_INTENT_EVENT,
  type PromptIntent,
  type TargetSelectionIntent,
} from "./events";

beforeAll(async () => {
  await import("./lens-prompt-settings");
  await import("./lens-target-selection-view");
  await import("./lens-overlay-view");
});

afterEach(() => {
  document.body.replaceChildren();
});

describe("component property and event contracts", () => {
  it("leaves target-selection entrance motion outside the WebView content", async () => {
    const element = document.createElement("lens-target-selection-view") as HTMLElement & {
      model: TargetSelectionViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = {
      platform: "macos",
      lens: {
        operation_id: "operation",
        stage: "selecting",
        selection: {
          selection_id: "operation",
          stage: "reviewing",
          maximum_targets: 4,
          items: [],
        },
        output_blocks: [],
      },
      pending: false,
      message: "",
    };
    document.body.append(element);
    await element.updateComplete;

    const shell = element.shadowRoot?.querySelector<HTMLElement>(".target-selection-shell");
    expect(shell?.hasAttribute("data-entrance")).toBe(false);
  });

  it("keeps the prompt draft local and emits a composed semantic save intent", async () => {
    const element = document.createElement("lens-prompt-settings") as HTMLElement & {
      responsePrompt: string;
      updateComplete: Promise<boolean>;
    };
    element.responsePrompt = "Original prompt";
    const received = vi.fn<EventListener>();
    document.body.addEventListener(PROMPT_INTENT_EVENT, received, { once: true });
    document.body.append(element);
    await element.updateComplete;

    const textarea = element.querySelector<HTMLTextAreaElement>("textarea");
    expect(textarea?.value).toBe("Original prompt");
    if (!textarea) throw new Error("Prompt textarea is missing");
    textarea.value = "Updated prompt";
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;
    element.querySelector<HTMLButtonElement>('button[type="submit"]')?.click();

    expect(received).toHaveBeenCalledOnce();
    const event = received.mock.calls[0]?.[0] as CustomEvent<PromptIntent> | undefined;
    expect(event?.detail).toEqual({ type: "save", responsePrompt: "Updated prompt" });
    expect(event?.bubbles).toBe(true);
    expect(event?.composed).toBe(true);
  });

  it("accepts the authoritative prompt value during an explicit reset", async () => {
    const element = document.createElement("lens-prompt-settings") as HTMLElement & {
      responsePrompt: string;
      synchronization: "preserve-local-draft" | "accept-parent-value";
      updateComplete: Promise<boolean>;
    };
    element.responsePrompt = "Original prompt";
    document.body.append(element);
    await element.updateComplete;

    const textarea = element.querySelector<HTMLTextAreaElement>("textarea");
    if (!textarea) throw new Error("Prompt textarea is missing");
    textarea.value = "Unsaved edit";
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;

    element.synchronization = "accept-parent-value";
    element.responsePrompt = "Built-in prompt";
    await element.updateComplete;

    expect(textarea.value).toBe("Built-in prompt");
    expect(element.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled).toBe(true);
  });

  it("starts remove intent immediately and retains the departing card through its motion", async () => {
    const item: LensTargetSelectionItem = {
      id: "macos:com.apple.Safari:417",
      window: {
        window_id: 417,
        title: "Fixture",
        application_name: "Safari",
        bundle_id: "com.apple.Safari",
        pid: 417,
        frame: { x: 0, y: 0, width: 800, height: 600 },
      },
      preview_uri: "lens://selection/operation/window/417",
    };
    const model: TargetSelectionViewModel = {
      platform: "macos",
      lens: {
        operation_id: "operation",
        stage: "selecting",
        selection: {
          selection_id: "operation",
          stage: "reviewing",
          maximum_targets: 4,
          items: [item],
        },
        output_blocks: [],
      },
      pending: false,
      message: "",
    };
    const element = document.createElement("lens-target-selection-view") as HTMLElement & {
      model: TargetSelectionViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = model;
    const received = vi.fn<EventListener>();
    document.body.addEventListener(TARGET_SELECTION_INTENT_EVENT, received, {
      once: true,
    });
    document.body.append(element);
    await element.updateComplete;
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector('[aria-label^="Remove Safari"]')).not.toBeNull();
    });

    element.shadowRoot?.querySelector<HTMLButtonElement>('[aria-label^="Remove Safari"]')?.click();

    await element.updateComplete;
    expect(received).toHaveBeenCalledOnce();
    const event = received.mock.calls[0]?.[0] as CustomEvent<TargetSelectionIntent> | undefined;
    expect(event?.detail).toEqual({ type: "remove", targetId: item.id });
    expect(event?.bubbles).toBe(true);
    expect(event?.composed).toBe(true);

    element.model = {
      ...model,
      pending: true,
      lens: {
        ...model.lens,
        selection: { ...model.lens.selection!, items: [] },
      },
    };
    await element.updateComplete;

    const departingCard = element.shadowRoot?.querySelector<HTMLElement>(
      `[data-target-id="${item.id}"]`,
    );
    expect(departingCard?.dataset.motion).toBe("removing");
    departingCard?.dispatchEvent(new Event("animationend", { bubbles: true }));
    await element.updateComplete;
    expect(element.shadowRoot?.querySelector(`[data-target-id="${item.id}"]`)).toBeNull();
  });

  it("declares the inverse card motion when one reviewed target is added", async () => {
    const first: LensTargetSelectionItem = {
      id: "macos:com.apple.Safari:417",
      window: {
        window_id: 417,
        title: "Fixture",
        application_name: "Safari",
        bundle_id: "com.apple.Safari",
        pid: 417,
        frame: { x: 0, y: 0, width: 800, height: 600 },
      },
      preview_uri: "lens://selection/operation/window/417",
    };
    const second: LensTargetSelectionItem = {
      id: "macos:com.apple.TextEdit:512",
      window: {
        window_id: 512,
        title: "Notes",
        application_name: "TextEdit",
        bundle_id: "com.apple.TextEdit",
        pid: 512,
        frame: { x: 80, y: 80, width: 600, height: 500 },
      },
      preview_uri: "lens://selection/operation/window/512",
    };
    const selection = {
      selection_id: "operation",
      stage: "reviewing" as const,
      maximum_targets: 4,
      items: [first],
    };
    const element = document.createElement("lens-target-selection-view") as HTMLElement & {
      model: TargetSelectionViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = {
      platform: "macos",
      lens: {
        operation_id: "operation",
        stage: "selecting",
        selection,
        output_blocks: [],
      },
      pending: true,
      message: "",
    };
    document.body.append(element);
    await element.updateComplete;

    element.model = {
      ...element.model,
      lens: {
        ...element.model.lens,
        selection: { ...selection, items: [first, second] },
      },
    };
    await element.updateComplete;

    const added = element.shadowRoot?.querySelector<HTMLElement>(`[data-target-id="${second.id}"]`);
    expect(added?.dataset.motion).toBe("adding");
    added?.dispatchEvent(new Event("animationend", { bubbles: true }));
    await element.updateComplete;
    expect(added?.dataset.motion).toBe("settled");
  });

  it("keeps cancellation available while an Agent command is pending", async () => {
    const model: OverlayViewModel = {
      platform: "macos",
      lens: {
        operation_id: "operation",
        stage: "transforming",
        output_blocks: [],
        agent: {
          run_id: "run",
          kind: "codex",
          adapter_name: "Codex",
          adapter_version: "1",
          auth_methods: [],
          received_updates: 0,
        },
      },
      pending: true,
      cancelPending: false,
      message: "",
    };
    const element = document.createElement("lens-overlay-view") as HTMLElement & {
      model: OverlayViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = model;
    document.body.append(element);
    await element.updateComplete;

    const cancel = Array.from(element.shadowRoot?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "Cancel",
    );
    expect(cancel?.disabled).toBe(false);
    const progressRegion = element.shadowRoot?.querySelector(".lens-progress-region");
    expect(progressRegion?.getAttribute("role")).toBe("status");
    expect(progressRegion?.parentElement?.classList.contains("overlay-footer")).toBe(true);
    expect(element.shadowRoot?.querySelector(".overlay-main .lens-progress-region")).toBeNull();
    expect(element.shadowRoot?.querySelector(".lens-progress-snackbar")?.textContent).toContain(
      "Transforming content",
    );
    expect(element.shadowRoot?.querySelector(".lens-progress-snackbar")?.textContent).toContain(
      "Translation updates",
    );
    const output = element.shadowRoot?.querySelector<
      HTMLElement & { updateComplete: Promise<boolean> }
    >("lens-agent-output");
    await output?.updateComplete;
    expect(output?.querySelector(".loading-state")).toBeNull();
    expect(output?.querySelector(".empty-state")?.textContent).toContain(
      "translation will appear here",
    );

    element.model = { ...model, cancelPending: true };
    await element.updateComplete;
    expect(cancel?.disabled).toBe(true);
  });
});
