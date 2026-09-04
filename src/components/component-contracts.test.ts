// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import appIconUrl from "../../src-tauri/icons/icon-macos.svg?url";
import type { OverlayViewModel, TargetSelectionViewModel } from "../application/view-models";
import type {
  AgentPromptTemplate,
  LensRepresentation,
  LensState,
  LensTargetSelectionItem,
} from "../types";
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

function promptTemplate(common: string): AgentPromptTemplate {
  return {
    schema_version: 1,
    common,
    full_projection: "Use the initial projection.",
    source_checkpoint: "Replace revision {base_revision} with {target_revision}.",
    current_projection_retry: "Retry revision {applied_revision}.",
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
      agentPromptTemplate: AgentPromptTemplate;
      updateComplete: Promise<boolean>;
    };
    element.agentPromptTemplate = promptTemplate("Original prompt\n\n{turn_instruction}");
    const received = vi.fn<EventListener>();
    document.body.addEventListener(PROMPT_INTENT_EVENT, received, { once: true });
    document.body.append(element);
    await element.updateComplete;

    const textarea = element.querySelector<HTMLTextAreaElement>("textarea");
    expect(textarea?.value).toBe("Original prompt\n\n{turn_instruction}");
    if (!textarea) throw new Error("Prompt textarea is missing");
    textarea.value = "Updated prompt\n\n{turn_instruction}";
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;
    element.querySelector<HTMLInputElement>('input[value="request"]')?.click();
    await element.updateComplete;
    expect(textarea.value).toBe("Use the initial projection.");
    element.querySelector<HTMLInputElement>('input[value="shared"]')?.click();
    await element.updateComplete;
    expect(textarea.value).toBe("Updated prompt\n\n{turn_instruction}");
    element.querySelector<HTMLButtonElement>('button[type="submit"]')?.click();

    expect(received).toHaveBeenCalledOnce();
    const event = received.mock.calls[0]?.[0] as CustomEvent<PromptIntent> | undefined;
    expect(event?.detail).toEqual({
      type: "save",
      agentPromptTemplate: promptTemplate("Updated prompt\n\n{turn_instruction}"),
    });
    expect(event?.bubbles).toBe(true);
    expect(event?.composed).toBe(true);
  });

  it("exposes the rendered prompt as one always-visible labeled section", async () => {
    const element = document.createElement("lens-prompt-settings") as HTMLElement & {
      agentPromptTemplate: AgentPromptTemplate;
      updateComplete: Promise<boolean>;
    };
    element.agentPromptTemplate = promptTemplate("Original prompt\n\n{turn_instruction}");
    document.body.append(element);
    await element.updateComplete;

    const preview = element.querySelector<HTMLElement>(".prompt-preview-section");
    const heading = preview?.querySelector("#prompt-preview-heading");
    expect(preview?.getAttribute("aria-labelledby")).toBe("prompt-preview-heading");
    expect(heading?.textContent).toBe("Rendered Prompt");
    expect(preview?.textContent).toContain("Shared Instructions + Initial Request");
    expect(preview?.querySelector('[aria-label="Rendered Agent instruction"]')).not.toBeNull();
    expect(preview?.querySelector("details, summary")).toBeNull();
  });

  it("inserts a declared prompt variable at the textarea caret and selects an existing one", async () => {
    const element = document.createElement("lens-prompt-settings") as HTMLElement & {
      agentPromptTemplate: AgentPromptTemplate;
      updateComplete: Promise<boolean>;
    };
    element.agentPromptTemplate = promptTemplate("Original prompt\n\n{turn_instruction}");
    document.body.append(element);
    await element.updateComplete;

    const textarea = element.querySelector<HTMLTextAreaElement>("textarea");
    if (!textarea) throw new Error("Prompt textarea is missing");
    textarea.value = "Prefix suffix";
    textarea.setSelectionRange(7, 7);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;

    const variable = element.querySelector<HTMLButtonElement>('[data-variable="turn_instruction"]');
    expect(variable?.dataset.state).toBe("available");
    variable?.click();
    await element.updateComplete;
    await Promise.resolve();

    expect(textarea.value).toBe("Prefix {turn_instruction}suffix");
    expect(document.activeElement).toBe(textarea);
    expect(textarea.selectionStart).toBe("Prefix {turn_instruction}".length);
    expect(textarea.selectionEnd).toBe("Prefix {turn_instruction}".length);

    const inserted = element.querySelector<HTMLButtonElement>('[data-variable="turn_instruction"]');
    expect(inserted?.dataset.state).toBe("inserted");
    inserted?.click();
    await element.updateComplete;
    await Promise.resolve();

    expect(textarea.value).toBe("Prefix {turn_instruction}suffix");
    expect(textarea.value.slice(textarea.selectionStart, textarea.selectionEnd)).toBe(
      "{turn_instruction}",
    );
  });

  it("accepts the authoritative prompt value during an explicit reset", async () => {
    const element = document.createElement("lens-prompt-settings") as HTMLElement & {
      agentPromptTemplate: AgentPromptTemplate;
      synchronization: "preserve-local-draft" | "accept-parent-value";
      updateComplete: Promise<boolean>;
    };
    element.agentPromptTemplate = promptTemplate("Original prompt\n\n{turn_instruction}");
    document.body.append(element);
    await element.updateComplete;

    const textarea = element.querySelector<HTMLTextAreaElement>("textarea");
    if (!textarea) throw new Error("Prompt textarea is missing");
    textarea.value = "Unsaved edit\n\n{turn_instruction}";
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;

    element.synchronization = "accept-parent-value";
    await element.updateComplete;
    expect(textarea.value).toBe("Original prompt\n\n{turn_instruction}");

    textarea.value = "Edit before the reset snapshot\n\n{turn_instruction}";
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;
    element.agentPromptTemplate = promptTemplate("Built-in prompt\n\n{turn_instruction}");
    await element.updateComplete;

    expect(textarea.value).toBe("Built-in prompt\n\n{turn_instruction}");
    expect(element.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled).toBe(true);

    textarea.value = "New unsaved edit\n\n{turn_instruction}";
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;
    element.agentPromptTemplate = promptTemplate("Built-in prompt\n\n{turn_instruction}");
    await element.updateComplete;

    expect(textarea.value).toBe("New unsaved edit\n\n{turn_instruction}");
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
    const appIcon = element.shadowRoot?.querySelector<HTMLImageElement>(".overlay-app-icon");
    expect(appIcon?.getAttribute("src")).toBe(appIconUrl);
    expect(appIcon?.alt).toBe("");
    const progressRegion = element.shadowRoot?.querySelector(".lens-progress-region");
    const progressStatus = progressRegion?.querySelector(".lens-status-announcement");
    expect(progressStatus?.getAttribute("role")).toBe("status");
    expect(progressStatus?.getAttribute("aria-live")).toBe("polite");
    expect(progressRegion?.parentElement?.classList.contains("overlay-shell")).toBe(true);
    expect(progressRegion?.closest(".overlay-main, .overlay-footer, .lens-panel")).toBeNull();
    expect(element.shadowRoot?.querySelector(".lens-progress-snackbar")?.textContent).toContain(
      "Transforming content",
    );
    expect(element.shadowRoot?.querySelector(".lens-progress-snackbar")?.textContent).toContain(
      "Interpretation updates",
    );
    const output = element.shadowRoot?.querySelector<
      HTMLElement & { updateComplete: Promise<boolean> }
    >("lens-agent-output");
    await output?.updateComplete;
    expect(output?.querySelector(".loading-state")).toBeNull();
    expect(output?.querySelector(".empty-state")?.textContent).toContain(
      "interpretation will appear here",
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

  it("describes missing Agent output as an interpretation", async () => {
    const element = document.createElement("lens-agent-output") as HTMLElement & {
      lens: LensState;
      updateComplete: Promise<boolean>;
    };
    element.lens = { stage: "failed", output_blocks: [] };
    document.body.append(element);
    await element.updateComplete;
    expect(element.querySelector(".empty-state")?.textContent).toBe(
      "The Agent did not produce an interpretation.",
    );
  });

  it("settles an atomic representation without making the interpretation a live region", async () => {
    const element = document.createElement("lens-agent-output") as HTMLElement & {
      lens: LensState;
      updateComplete: Promise<boolean>;
    };
    element.lens = {
      ...liveLens(representation("representation-1", 1, "Published interpretation")),
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

    expect(output?.textContent).toContain("Published interpretation");
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
        ...liveLens(representation("representation-1", 1, "Interpretation")),
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

  it("resets to Interpretation only when the operation changes", async () => {
    const element = document.createElement("lens-overlay-view") as HTMLElement & {
      model: OverlayViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = {
      platform: "macos",
      lens: liveLens(representation("representation-1", 1, "First result")),
      pending: false,
      cancelPending: false,
      message: "",
    };
    document.body.append(element);
    await element.updateComplete;
    element.shadowRoot?.querySelector<HTMLButtonElement>("#source-tab")?.click();
    await element.updateComplete;

    element.model = {
      ...element.model,
      lens: liveLens(representation("representation-2", 2, "Updated result")),
    };
    await element.updateComplete;
    expect(element.shadowRoot?.querySelector('[role="tabpanel"]')?.id).toBe("source-panel");

    element.model = {
      ...element.model,
      lens: { operation_id: "next-operation", stage: "connecting", output_blocks: [] },
    };
    await element.updateComplete;
    expect(element.shadowRoot?.querySelector('[role="tabpanel"]')?.id).toBe("interpretation-panel");
    expect(
      element.shadowRoot?.querySelector("#interpretation-tab")?.getAttribute("aria-selected"),
    ).toBe("true");
  });

  it.each([
    { top: 200, initialHeight: 1000, nextHeight: 1000, expected: 200 },
    { top: 700, initialHeight: 1400, nextHeight: 500, expected: 100 },
    { top: 600, initialHeight: 1000, nextHeight: 1400, expected: 1000 },
  ])(
    "restores Interpretation scroll position for $top -> $expected",
    async ({ top, initialHeight, nextHeight, expected }) => {
      const element = document.createElement("lens-overlay-view") as HTMLElement & {
        model: OverlayViewModel;
        updateComplete: Promise<boolean>;
      };
      element.model = {
        platform: "macos",
        lens: liveLens(representation("representation-1", 1, "First result")),
        pending: false,
        cancelPending: false,
        message: "",
      };
      document.body.append(element);
      await vi.waitFor(() =>
        expect(element.shadowRoot?.querySelector(".lens-output")).not.toBeNull(),
      );
      const output = element.shadowRoot!.querySelector<HTMLElement>(".lens-output")!;
      let height = initialHeight;
      Object.defineProperties(output, {
        clientHeight: { get: () => 400 },
        scrollHeight: { get: () => height },
      });
      output.scrollTop = top;
      element.model = {
        ...element.model,
        lens: liveLens(representation("representation-2", 2, "Updated result")),
      };
      await element.updateComplete;
      height = nextHeight;
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      await vi.waitFor(() => {
        expect(element.shadowRoot?.querySelector(".lens-output")?.textContent).toContain(
          "Updated result",
        );
        expect(output.scrollTop).toBe(expected);
      });
    },
  );

  it.each([
    { top: 0, initialHeight: 400, expected: 0 },
    { top: 600, initialHeight: 1000, expected: 600 },
  ])(
    "preserves media reading position on settled replacement at $top",
    async ({ top, initialHeight, expected }) => {
      const withMedia = (id: string, revision: number): LensRepresentation => ({
        ...representation(id, revision, "Image explanation"),
        output_blocks: [
          { type: "image", mime_type: "image/png", data: "aA==" },
          { type: "markdown", text: "Image explanation" },
        ],
      });
      const element = document.createElement("lens-overlay-view") as HTMLElement & {
        model: OverlayViewModel;
        updateComplete: Promise<boolean>;
      };
      element.model = {
        platform: "macos",
        lens: liveLens(withMedia("media-1", 1)),
        pending: false,
        cancelPending: false,
        message: "",
      };
      document.body.append(element);
      await vi.waitFor(() =>
        expect(element.shadowRoot?.querySelector(".lens-output.has-media")).not.toBeNull(),
      );
      const output = element.shadowRoot!.querySelector<HTMLElement>(".lens-output")!;
      let height = initialHeight;
      Object.defineProperties(output, {
        clientHeight: { get: () => 400 },
        scrollHeight: { get: () => height },
      });
      output.scrollTop = top;
      element.model = { ...element.model, lens: liveLens(withMedia("media-2", 2)) };
      await element.updateComplete;
      height = 1400;
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      await vi.waitFor(() => expect(output.scrollTop).toBe(expected));
    },
  );

  it("applies each complete replacement automatically while preserving Interpretation focus", async () => {
    const first = representation("representation-1", 1, "Interpretation one");
    const second = representation("representation-2", 2, "Interpretation two");
    const third = representation("representation-3", 3, "Interpretation three");
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
        "Interpretation one",
      );
    });

    const panel = element.shadowRoot?.querySelector<HTMLElement>("#interpretation-panel");
    panel?.focus();
    expect(element.shadowRoot?.activeElement).toBe(panel);

    element.model = { ...element.model, lens: liveLens(second) };
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector(".lens-output")?.textContent).toContain(
        "Interpretation two",
      );
    });
    element.model = { ...element.model, lens: liveLens(third) };
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector(".lens-output")?.textContent).toContain(
        "Interpretation three",
      );
    });
    expect(element.shadowRoot?.querySelector(".lens-output")?.textContent).not.toContain(
      "Interpretation two",
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
      lens: liveLens(representation("representation-1", 1, "Interpretation")),
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

describe("progress notification visibility", () => {
  async function mount(lens: LensState) {
    const element = document.createElement("lens-overlay-view") as HTMLElement & {
      model: OverlayViewModel;
      updateComplete: Promise<boolean>;
    };
    element.model = { platform: "macos", lens, pending: false, cancelPending: false, message: "" };
    document.body.append(element);
    await element.updateComplete;
    return element;
  }

  it("dismisses and reopens the same notification without cancelling or moving content", async () => {
    const element = await mount({
      operation_id: "operation",
      stage: "transforming",
      output_blocks: [{ type: "markdown", text: "Continuing interpretation." }],
    });
    const root = element.shadowRoot!;
    const received = vi.fn<EventListener>();
    element.addEventListener(OVERLAY_INTENT_EVENT, received);
    const toggle = root.querySelector<HTMLButtonElement>(".overlay-status-toggle")!;
    const announcement = root.querySelector(".lens-status-announcement");
    expect(toggle.getAttribute("aria-controls")).toBe("lens-progress-notification");
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    const output = root.querySelector<HTMLElement>(".lens-output")!;
    output.scrollTop = 120;
    root.querySelector<HTMLButtonElement>(".lens-progress-dismiss")!.click();
    await element.updateComplete;
    expect(root.querySelector(".lens-progress-snackbar")).toBeNull();
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(root.activeElement).toBe(toggle);
    expect(root.querySelector(".lens-status-announcement")).toBe(announcement);
    expect(root.querySelectorAll('[role="status"]')).toHaveLength(1);
    expect(output.scrollTop).toBe(120);
    element.model = { ...element.model, lens: { ...element.model.lens } };
    await element.updateComplete;
    expect(root.querySelector(".lens-progress-snackbar")).toBeNull();
    root.querySelector<HTMLButtonElement>("#source-tab")!.click();
    await element.updateComplete;
    expect(root.querySelector(".lens-progress-snackbar")).toBeNull();
    toggle.click();
    await element.updateComplete;
    expect(root.querySelector(".lens-progress-snackbar")).not.toBeNull();
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    toggle.click();
    await element.updateComplete;
    expect(root.querySelector(".lens-progress-snackbar")).toBeNull();
    expect(received).not.toHaveBeenCalled();
  });

  it("opens a changed notification or a new operation after dismissal", async () => {
    const lens = liveLens(representation("published", 1, "Retained interpretation."));
    const element = await mount({
      ...lens,
      live: {
        ...lens.live!,
        health: "unavailable",
        freshness: "unverified",
        error: "Capture unavailable.",
      },
    });
    const root = element.shadowRoot!;
    root.querySelector<HTMLButtonElement>(".lens-progress-dismiss")!.click();
    await element.updateComplete;
    element.model = {
      ...element.model,
      lens: {
        ...element.model.lens,
        live: { ...element.model.lens.live!, error: "Capture permission changed." },
      },
    };
    await element.updateComplete;
    expect(root.querySelector(".lens-progress-snackbar")?.textContent).toContain(
      "Capture permission changed.",
    );
    root.querySelector<HTMLButtonElement>(".lens-progress-dismiss")!.click();
    await element.updateComplete;
    element.model = {
      ...element.model,
      lens: { ...element.model.lens, operation_id: "new-operation" },
    };
    await element.updateComplete;
    expect(root.querySelector(".lens-progress-snackbar")).not.toBeNull();
  });

  it("shows quiet status details only when requested from the footer", async () => {
    const element = await mount(liveLens(representation("published", 1, "Interpretation.")));
    const root = element.shadowRoot!;
    const toggle = root.querySelector<HTMLButtonElement>(".overlay-status-toggle")!;
    expect(root.querySelector(".lens-progress-snackbar")).toBeNull();
    expect(toggle.textContent).toContain("Watching");
    toggle.click();
    await element.updateComplete;
    expect(root.querySelector(".lens-progress-snackbar")?.textContent).toContain("Watching");
    element.model = { ...element.model, lens: { ...element.model.lens, live: undefined } };
    await element.updateComplete;
    expect(root.querySelector(".overlay-status-toggle")).toBeNull();
    expect(root.querySelector(".lens-progress-snackbar")).toBeNull();
    expect(root.querySelector(".overlay-footer-status")?.textContent).toContain(
      "Transformation complete",
    );
  });
});
