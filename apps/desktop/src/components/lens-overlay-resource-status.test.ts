// @vitest-environment jsdom
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import type { OverlayViewModel } from "../application/view-models";
import type { LensOverlayView } from "./lens-overlay-view";
import type { LensExtractionDiagnostics } from "./lens-extraction-diagnostics";

beforeAll(async () => {
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  globalThis.requestAnimationFrame ??= (callback: FrameRequestCallback) =>
    window.setTimeout(() => callback(performance.now()), 0);
  globalThis.cancelAnimationFrame ??= (handle: number) => window.clearTimeout(handle);
  HTMLElement.prototype.scrollTo ??= () => undefined;
  await import("./lens-overlay-view");
  await import("./lens-agent-output");
  await import("./lens-extraction-diagnostics");
});
afterEach(() => document.body.replaceChildren());

function model(): OverlayViewModel {
  return {
    platform: "macos",
    lens: {
      operation_id: "operation",
      stage: "transforming",
      prompt_execution_revision: 1,
      output_blocks: [{ type: "markdown", text: "Existing interpretation" }],
      response_history: { responses: [], retained_bytes: 0, capacity_reached: false },
    },
    sourceMetadata: { has_input: true, quality: "full", projection_has_loss: false },
    sourceResource: { stage: "ready" },
    outputResource: { stage: "ready" },
    pending: false,
    cancelPending: false,
    message: "",
  };
}

async function mount() {
  const view = document.createElement("lens-overlay-view") as LensOverlayView;
  view.active = true;
  view.model = model();
  document.body.append(view);
  await view.updateComplete;
  return view;
}

async function select(view: LensOverlayView, tab: "source" | "diagnostics") {
  view.shadowRoot!.querySelector<HTMLButtonElement>(`#${tab}-tab`)!.click();
  await view.updateComplete;
}

it("keeps the Interpretation host while pending output uses the existing progress surface", async () => {
  const view = await mount();
  const output = view.shadowRoot!.querySelector("lens-agent-output");
  view.model = { ...view.model!, outputResource: { stage: "loading" } };
  await view.updateComplete;
  const panel = view.shadowRoot!.querySelector("#interpretation-panel")!;
  expect(panel.getAttribute("aria-busy")).toBe("true");
  expect(panel.textContent).not.toContain("Loading output");
  expect(panel.querySelector("lens-agent-output")).toBe(output);
  expect(panel.querySelector(":scope > p")).toBeNull();
  expect(view.shadowRoot!.querySelector(".lens-status-announcement")?.textContent).toContain(
    "Transforming content",
  );

  view.model = { ...view.model!, outputResource: { stage: "ready" } };
  await view.updateComplete;
  expect(panel.getAttribute("aria-busy")).toBe("false");
});

it("withholds missing-source prose during pending source retrieval and clears prior source content", async () => {
  const view = await mount();
  view.model = {
    ...view.model!,
    lens: {
      ...view.model!.lens,
      input: {
        schema_version: 3,
        context_id: "old-source",
        context_revision: 1,
        sources: [],
        media: [],
        media_omissions: [],
        quality: "full",
      },
    },
  };
  await select(view, "source");
  expect(view.shadowRoot!.querySelector(".source-content")?.textContent).toContain("old-source");

  view.model = {
    ...view.model!,
    lens: { ...view.model!.lens, input: undefined, context: undefined },
    sourceResource: { stage: "loading" },
  };
  await view.updateComplete;
  const panel = view.shadowRoot!.querySelector("#source-panel")!;
  expect(panel.getAttribute("aria-busy")).toBe("true");
  expect(panel.textContent).not.toContain("Loading source");
  expect(panel.textContent).not.toContain("old-source");
  expect(panel.textContent).not.toContain("No normalized source");

  view.model = {
    ...view.model!,
    sourceResource: { stage: "idle" },
    sourceMetadata: { ...view.model!.sourceMetadata!, has_input: false, quality: null },
  };
  await view.updateComplete;
  expect(panel.getAttribute("aria-busy")).toBe("false");
  expect(panel.textContent).toContain("No normalized source");
});

it.each([
  ["source", true],
  ["diagnostics", true],
  ["source", false],
  ["diagnostics", false],
] as const)(
  "treats deferred %s demand as pending while capture details exist (has input: %s)",
  async (tab, hasInput) => {
    const view = await mount();
    view.model = {
      ...view.model!,
      sourceResource: { stage: "idle" },
      sourceMetadata: { ...view.model!.sourceMetadata!, has_input: hasInput },
    };
    await select(view, tab);
    const panel = view.shadowRoot!.querySelector(`#${tab}-panel`)!;
    const diagnostics = panel.querySelector<LensExtractionDiagnostics>(
      "lens-extraction-diagnostics",
    );
    await diagnostics?.updateComplete;
    expect(panel.getAttribute("aria-busy")).toBe("true");
    expect(panel.textContent).not.toContain("No normalized source");
    expect(panel.textContent).not.toContain("No extraction diagnostics are available");
    expect(panel.textContent).not.toContain("Loading");
  },
);

it.each(["interpretation", "source", "diagnostics"] as const)(
  "keeps %s resource failure and retry inside the content layout",
  async (tab) => {
    const view = await mount();
    const retry = vi.fn<NonNullable<LensOverlayView["retrySnapshotResource"]>>();
    view.retrySnapshotResource = retry;
    const kind = tab === "interpretation" ? "output" : "source";
    view.model = {
      ...view.model!,
      [`${kind}Resource`]: { stage: "failed", message: "Native read failed" },
    };
    if (tab !== "interpretation") await select(view, tab);
    else await view.updateComplete;
    const panel = view.shadowRoot!.querySelector(`#${tab}-panel`)!;
    const error = panel.querySelector(".lens-content.lens-resource-error")!;
    expect(panel.getAttribute("aria-busy")).toBe("false");
    expect(error.querySelector('[role="alert"]')?.textContent).toContain("Native read failed");
    expect(panel.querySelector(":scope > p")).toBeNull();
    error.querySelector<HTMLButtonElement>("button")!.click();
    expect(retry).toHaveBeenCalledWith(kind);
  },
);

it("keeps authoritative preparation evidence visible while Diagnostics bodies are pending", async () => {
  const view = await mount();
  view.model = {
    ...view.model!,
    sourceResource: { stage: "loading" },
    lens: { ...view.model!.lens, projection: { revision: 5, digest: "digest" } },
  };
  await select(view, "diagnostics");
  const panel = view.shadowRoot!.querySelector("#diagnostics-panel")!;
  const diagnostics = panel.querySelector<LensExtractionDiagnostics>(
    "lens-extraction-diagnostics",
  )!;
  await diagnostics.updateComplete;
  expect(panel.getAttribute("aria-busy")).toBe("true");
  expect(panel.textContent).not.toContain("Loading");
  expect(panel.textContent).not.toContain("Details loading");
  expect(panel.textContent).not.toContain("No extraction diagnostics are available");
  expect(panel.textContent).toContain("No text, resource, or document content was omitted");
});
