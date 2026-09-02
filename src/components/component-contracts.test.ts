// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { OverlayViewModel, TargetSelectionViewModel } from "../application/view-models";
import type { LensRepresentation, LensState, LensTargetSelectionItem } from "../types";
import {
  OVERLAY_INTENT_EVENT,
  PROMPT_INTENT_EVENT,
  TARGET_SELECTION_INTENT_EVENT,
  type PromptIntent,
  type OverlayIntent,
  type TargetSelectionIntent,
} from "./events";

function representation(id: string, revision: number, text: string): LensRepresentation {
  return {
    representation_id: id,
    context_id: "operation",
    context_revision: revision,
    projection: { revision, digest: `sha256:projection-${revision}` },
    run_id: `run-${revision}`,
    output_blocks: [{ type: "markdown", text }],
  };
}

function liveLens(current: LensRepresentation): LensState {
  return {
    operation_id: "operation",
    stage: "completed",
    output_blocks: [{ type: "markdown", text: "Compatibility output" }],
    representation: current,
    live: {
      lifecycle: "watching",
      health: "healthy",
      freshness: "current",
      agent_refresh_interval_seconds: 180,
    },
  };
}

beforeAll(async () => {
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  globalThis.requestAnimationFrame ??= (callback: FrameRequestCallback) =>
    window.setTimeout(() => callback(performance.now()), 0);
  globalThis.cancelAnimationFrame ??= (handle: number) => window.clearTimeout(handle);
  HTMLElement.prototype.scrollTo ??= () => undefined;
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
    const progressStatus = progressRegion?.querySelector(".lens-status-announcement");
    expect(progressStatus?.getAttribute("role")).toBe("status");
    expect(progressStatus?.getAttribute("aria-live")).toBe("polite");
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

  it("starts the initial Agent turn automatically and exposes retry only after failure", async () => {
    const input: NonNullable<LensState["input"]> = {
      schema_version: 3,
      context_id: "operation",
      context_revision: 1,
      sources: [],
      media: [],
      media_omissions: [],
      quality: "full",
    };
    const readyModel: OverlayViewModel = {
      platform: "macos",
      lens: {
        operation_id: "operation",
        stage: "ready",
        input,
        output_blocks: [],
        live: {
          lifecycle: "watching",
          health: "healthy",
          freshness: "none",
          agent_refresh_interval_seconds: 180,
        },
      },
      pending: false,
      cancelPending: false,
      message: "",
    };
    const element = document.createElement("lens-overlay-view") as HTMLElement & {
      model: OverlayViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = readyModel;
    document.body.append(element);
    await element.updateComplete;

    expect(element.shadowRoot?.textContent).not.toContain("Transform with Agent");
    expect(element.shadowRoot?.textContent).not.toContain("Retry with Agent");
    expect(element.shadowRoot?.querySelector(".lens-progress-snackbar")?.textContent).toContain(
      "Preparing Agent",
    );
    expect(element.shadowRoot?.querySelector(".lens-progress-snackbar")?.textContent).toContain(
      "starts automatically",
    );

    const received = vi.fn<EventListener>();
    document.body.addEventListener(OVERLAY_INTENT_EVENT, received, { once: true });
    element.model = {
      ...readyModel,
      lens: {
        ...readyModel.lens,
        stage: "failed",
        error: "Agent transport failed.",
      },
    };
    await element.updateComplete;

    const retry = Array.from(element.shadowRoot?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "Retry with Agent",
    );
    expect(retry?.classList.contains("overlay-header-action")).toBe(true);
    expect(retry?.parentElement?.classList.contains("overlay-header-actions")).toBe(true);
    expect(
      Array.from(element.shadowRoot?.querySelectorAll(".overlay-main button") ?? []).some(
        (button) => button.textContent?.trim() === "Retry with Agent",
      ),
    ).toBe(false);
    retry?.click();
    expect(received).toHaveBeenCalledOnce();
    const event = received.mock.calls[0]?.[0] as CustomEvent<OverlayIntent> | undefined;
    expect(event?.detail).toEqual({ type: "retry" });
  });

  it("settles an atomic representation without making the translation a live region", async () => {
    const element = document.createElement("lens-agent-output") as HTMLElement & {
      lens: LensState;
      updateComplete: Promise<boolean>;
    };
    element.lens = {
      ...liveLens(representation("representation-1", 1, "Published translation")),
      stage: "transforming",
      output_blocks: [{ type: "markdown", text: "Unpublished stream" }],
    };
    document.body.append(element);
    await element.updateComplete;

    const output = element.querySelector(".lens-output");
    const markdown = element.querySelector<
      HTMLElement & {
        state: { operationId?: string; phase: string };
        updateComplete: Promise<boolean>;
      }
    >("lens-markdown");
    await markdown?.updateComplete;

    expect(output?.textContent).toContain("Published translation");
    expect(output?.textContent).not.toContain("Unpublished stream");
    expect(output?.hasAttribute("aria-live")).toBe(false);
    expect(markdown?.state).toMatchObject({
      operationId: "representation-1:0",
      phase: "settled",
    });
  });

  it("renders the latest title facts without changing the selected target identity", async () => {
    const target = {
      id: "macos:com.apple.Safari:417",
      identity: {
        window_id: 417,
        bundle_id: "com.apple.Safari",
        pid: 417,
      },
      facts_revision: 1,
      facts: {
        title: "Picker title",
        application_name: "Safari",
        frame: { x: 0, y: 0, width: 800, height: 600 },
      },
    };
    const element = document.createElement("lens-overlay-view") as HTMLElement & {
      model: OverlayViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = {
      platform: "macos",
      lens: {
        ...liveLens(representation("representation-1", 1, "Translation")),
        target_set: {
          schema_version: 2,
          selection_id: "operation",
          targets: [target],
        },
      },
      pending: false,
      cancelPending: false,
      message: "",
    };
    document.body.append(element);
    await element.updateComplete;

    const sourceTargets = element.shadowRoot?.querySelector(".overlay-source-targets");
    expect(sourceTargets?.getAttribute("title")).toBe("Safari — Picker title");

    element.model = {
      ...element.model,
      lens: {
        ...element.model.lens,
        target_set: {
          ...element.model.lens.target_set!,
          targets: [
            {
              ...target,
              facts_revision: 2,
              facts: { ...target.facts, title: "Current title" },
            },
          ],
        },
      },
    };
    await element.updateComplete;

    expect(sourceTargets?.getAttribute("title")).toBe("Safari — Current title");
    expect(element.model.lens.target_set?.targets[0]?.id).toBe(target.id);
  });

  it("applies each complete replacement automatically while preserving Translation focus", async () => {
    const first = representation("representation-1", 1, "Translation one");
    const second = representation("representation-2", 2, "Translation two");
    const third = representation("representation-3", 3, "Translation three");
    const element = document.createElement("lens-overlay-view") as HTMLElement & {
      model: OverlayViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = {
      platform: "macos",
      lens: liveLens(first),
      pending: false,
      cancelPending: false,
      message: "",
    };
    document.body.append(element);
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector(".lens-output")?.textContent).toContain(
        "Translation one",
      );
    });

    const panel = element.shadowRoot?.querySelector<HTMLElement>("#translation-panel");
    panel?.focus();
    expect(element.shadowRoot?.activeElement).toBe(panel);

    element.model = { ...element.model, lens: liveLens(second) };
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector(".lens-output")?.textContent).toContain(
        "Translation two",
      );
    });
    element.model = { ...element.model, lens: liveLens(third) };
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector(".lens-output")?.textContent).toContain(
        "Translation three",
      );
    });
    expect(element.shadowRoot?.querySelector(".lens-output")?.textContent).not.toContain(
      "Translation two",
    );
    await vi.waitFor(() => expect(element.shadowRoot?.activeElement).toBe(panel));
    expect(element.shadowRoot?.querySelector(".lens-update-action")).toBeNull();
  });

  it("exposes finite Pause and Resume intents while monitoring status stays polite", async () => {
    const element = document.createElement("lens-overlay-view") as HTMLElement & {
      model: OverlayViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = {
      platform: "macos",
      lens: liveLens(representation("representation-1", 1, "Translation")),
      pending: false,
      cancelPending: false,
      message: "",
    };
    const received: OverlayIntent[] = [];
    document.body.addEventListener(OVERLAY_INTENT_EVENT, (event) => {
      received.push((event as CustomEvent<OverlayIntent>).detail);
    });
    document.body.append(element);
    await element.updateComplete;

    const pause = Array.from(element.shadowRoot?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "Pause Updates",
    );
    pause?.click();
    expect(received).toContainEqual({ type: "pause" });
    expect(
      element.shadowRoot?.querySelector('[role="status"][aria-live="polite"]')?.textContent,
    ).toContain("Watching");

    element.model = {
      ...element.model,
      lens: {
        ...element.model.lens,
        live: {
          lifecycle: "paused",
          health: "healthy",
          freshness: "unverified",
          agent_refresh_interval_seconds: 180,
        },
      },
    };
    await element.updateComplete;
    const resume = Array.from(element.shadowRoot?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "Resume Updates",
    );
    resume?.click();
    expect(received).toContainEqual({ type: "resume" });
    expect(element.shadowRoot?.querySelector(".overlay-footer-status")?.textContent).toContain(
      "Paused",
    );
  });
});
