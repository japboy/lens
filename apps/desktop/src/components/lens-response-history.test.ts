// @vitest-environment jsdom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { LensAgentOutput } from "./lens-agent-output";
import type { LensOverlayView } from "./lens-overlay-view";
import type { LensResponseBlock } from "./lens-response-block";
import type { LensOutputMedia } from "./lens-output-media";
import { responseBlockIdentity } from "../application/response-history-controller";
import type {
  ResponseHistoryPresentation,
  ResponseManifest,
  LoadResponseBlock,
} from "../application/response-history-controller";
import type { LensOutputBlock, LensResponseHistory } from "../types";

let intersections: Array<{ callback: IntersectionObserverCallback; target: Element }> = [];
let resizes: Array<{ callback: ResizeObserverCallback; target: Element }> = [];
beforeAll(async () => {
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  HTMLElement.prototype.scrollTo ??= () => undefined;
  await import("./lens-agent-output");
  await import("./lens-extraction-diagnostics");
  await import("./lens-overlay-view");
  await import("./lens-session-controls");
});
beforeEach(() => {
  intersections = [];
  resizes = [];
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      constructor(private callback: IntersectionObserverCallback) {}
      observe(target: Element) {
        intersections.push({ callback: this.callback, target });
      }
      disconnect() {}
    },
  );
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(private callback: ResizeObserverCallback) {}
      observe(target: Element) {
        resizes.push({ callback: this.callback, target });
      }
      disconnect() {}
    },
  );
});
afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});
function visible(element: Element, show = true) {
  [...intersections]
    .reverse()
    .find((entry) => entry.target === element)!
    .callback(
      [{ target: element, isIntersecting: show } as IntersectionObserverEntry],
      {} as IntersectionObserver,
    );
}
function resize(element: Element, height: number) {
  [...resizes]
    .reverse()
    .find((entry) => entry.target === element)
    ?.callback(
      [{ target: element, contentRect: { height } } as ResizeObserverEntry],
      {} as ResizeObserver,
    );
}
function manifest(n: number): ResponseManifest {
  return {
    id: `r${n}`,
    sequence: n,
    blocks: [{ type: "markdown", block_index: 0, byte_length: 3 }],
  };
}
function history(count: number, operationId = "op"): ResponseHistoryPresentation {
  return {
    scopeId: operationId,
    responses: Array.from({ length: count }, (_, i) => manifest(i + 1)),
    media: [],
    htmlContents: new Map(),
    mediaErrors: new Map(),
    capacityReached: false,
  };
}
const load = vi.fn<LoadResponseBlock>(async (_op, response) => ({
  type: "markdown",
  text: `## ${response}\n\nPreserved **paragraph**.`,
}));
async function mount(count = 1) {
  const element = document.createElement("lens-agent-output") as LensAgentOutput;
  element.history = history(count);
  element.loadResponseBlock = load;
  document.body.append(element);
  await element.updateComplete;
  return element;
}

function nativeHistory(presentation: ResponseHistoryPresentation): LensResponseHistory {
  const responses = presentation.responses.map((response) => ({
    representation_id: response.id,
    context_id: presentation.scopeId,
    context_revision: response.sequence,
    projection: { revision: response.sequence, digest: "fixture" },
    run_id: `run-${response.sequence}`,
    prompt_execution_revision: 1,
    sequence: response.sequence,
    delivery: response.delivery,
    blocks: [...response.blocks],
    block_count: response.blocks.length,
    retained_bytes: response.blocks.reduce(
      (total, block) => total + ("byte_length" in block ? block.byte_length : 0),
      0,
    ),
  }));
  return {
    responses,
    retained_bytes: responses.reduce((total, response) => total + response.retained_bytes, 0),
    capacity_reached: presentation.capacityReached,
  };
}

function appendHistory(view: LensOverlayView, presentation: ResponseHistoryPresentation): void {
  view.model = {
    ...view.model!,
    lens: { ...view.model!.lens, response_history: nativeHistory(presentation) },
  };
  view.responseHistory = presentation;
}

async function mountOverlay(count = 1) {
  const view = document.createElement("lens-overlay-view") as LensOverlayView;
  view.active = true;
  view.model = {
    platform: "macos",
    pending: false,
    cancelPending: false,
    message: "",
    lens: {
      operation_id: "op",
      stage: "completed",
      prompt_execution_revision: 1,
      response_history: nativeHistory(history(count)),
      output_blocks: [],
      live: {
        lifecycle: "watching",
        health: "healthy",
        freshness: "current",
        agent_refresh_interval_seconds: 180,
      },
    },
  };
  view.responseHistory = history(count);
  view.loadResponseBlock = load;
  document.body.append(view);
  await view.updateComplete;
  const output = view.shadowRoot!.querySelector<LensAgentOutput>("lens-agent-output")!;
  await output.updateComplete;
  return view;
}
describe("Overlay response update notification", () => {
  it("keeps first completion quiet, counts only later arrivals, and separates dismissal from acknowledgement", async () => {
    const view = await mountOverlay(0);
    const root = view.shadowRoot!;
    view.responseHistory = history(1);
    await view.updateComplete;
    expect(root.querySelector(".lens-response-update")).toBeNull();
    view.responseHistory = history(3);
    await view.updateComplete;
    const host = root.querySelector("#lens-progress-notification")!;
    expect(host.getAttribute("data-pending-count")).toBe("2");
    expect(root.querySelector(".lens-response-update")?.textContent).toContain("2 new responses");
    root.querySelector<HTMLButtonElement>(".lens-progress-dismiss")!.click();
    await view.updateComplete;
    expect(root.querySelector(".lens-response-update")).toBeNull();
    expect(root.querySelector(".overlay-new-response-count")?.textContent).toBe("2 new responses");
    view.responseHistory = history(3);
    view.model = {
      ...view.model!,
      lens: { ...view.model!.lens, live: { ...view.model!.lens.live!, lifecycle: "paused" } },
    };
    await view.updateComplete;
    expect(root.querySelector(".overlay-status-toggle")?.getAttribute("aria-expanded")).toBe(
      "false",
    );
    root.querySelector<HTMLButtonElement>(".overlay-status-toggle")!.click();
    await view.updateComplete;
    expect(root.querySelector("#lens-progress-notification")).toBe(host);
    expect(host.getAttribute("data-pending-count")).toBe("2");
    view.model = { ...view.model!, lens: { ...view.model!.lens, operation_id: "replacement" } };
    view.responseHistory = undefined;
    await view.updateComplete;
    expect(root.querySelector(".overlay-new-response-count")).toBeNull();
    expect(root.querySelector(".lens-view-latest")).toBeNull();
  });

  it("acknowledges a captured narrative only after body presentation and leaves later arrivals pending", async () => {
    const view = await mountOverlay();
    let finish!: (block: LensOutputBlock) => void;
    view.loadResponseBlock = () =>
      new Promise((resolve) => {
        finish = resolve;
      });
    view.responseHistory = history(3);
    await view.updateComplete;
    const root = view.shadowRoot!;
    root.querySelector<HTMLButtonElement>(".lens-view-latest")!.click();
    await vi.waitFor(() => expect(finish).toBeDefined());
    expect(
      root.querySelector("#lens-progress-notification")?.getAttribute("data-pending-count"),
    ).toBe("2");
    view.responseHistory = history(4);
    await view.updateComplete;
    finish({ type: "markdown", text: "## Captured third response" });
    await vi.waitFor(() =>
      expect(
        root.querySelector("#lens-progress-notification")?.getAttribute("data-pending-count"),
      ).toBe("1"),
    );
    expect(root.activeElement?.getAttribute("data-response-sequence")).toBe("3");
    expect(root.querySelector('[data-response-sequence="3"] lens-markdown h2')?.textContent).toBe(
      "Captured third response",
    );
    expect(root.querySelector(".overlay-new-response-count")?.textContent).toBe("1 new response");
  });

  it("opens Interpretation from another tab and prefers the captured response media", async () => {
    const view = await mountOverlay();
    const response = history(2);
    const mediaId = responseBlockIdentity("op", "r2", 1);
    view.responseHistory = {
      ...response,
      responses: response.responses.map((item) =>
        item.id === "r2"
          ? {
              ...item,
              blocks: [
                ...item.blocks,
                { type: "image" as const, block_index: 1, mime_type: "image/png", byte_length: 4 },
              ],
            }
          : item,
      ),
      media: [
        { kind: "image", id: mediaId, source: "data:image/png;base64,aA==", mimeType: "image/png" },
      ],
    };
    await view.updateComplete;
    const root = view.shadowRoot!;
    const output = root.querySelector<LensAgentOutput>("lens-agent-output")!;
    await output.updateComplete;
    const media = output.querySelector<LensOutputMedia>("lens-output-media")!;
    await media.updateComplete;
    const retry = vi.fn<(id: string) => Promise<void>>(async () => undefined);
    view.retryResponseMedia = retry;
    const present = vi.spyOn(media, "presentMedia").mockResolvedValue(true);
    const exit = vi.spyOn(media, "exitFullscreen").mockResolvedValue(true);
    root.querySelector<HTMLButtonElement>("#source-tab")!.click();
    await view.updateComplete;
    root.querySelector<HTMLButtonElement>(".lens-view-latest")!.click();
    await vi.waitFor(() => expect(present).toHaveBeenCalledWith(mediaId));
    await vi.waitFor(() =>
      expect(
        root.querySelector("#lens-progress-notification")?.getAttribute("data-pending-count"),
      ).toBe("0"),
    );
    expect(exit).toHaveBeenCalled();
    expect(retry).toHaveBeenCalledWith(mediaId);
    expect(retry.mock.invocationCallOrder[0]).toBeLessThan(present.mock.invocationCallOrder[0]!);
    expect(root.querySelector("#interpretation-panel")).not.toBeNull();
  });

  it("keeps failed and obsolete navigation unacknowledged", async () => {
    const view = await mountOverlay();
    view.responseHistory = history(2);
    view.loadResponseBlock = async () => {
      throw new Error("offline");
    };
    await view.updateComplete;
    const root = view.shadowRoot!;
    root.querySelector<HTMLButtonElement>(".lens-view-latest")!.click();
    await vi.waitFor(() => expect(root.querySelector(".lens-update-error")).not.toBeNull());
    expect(
      root.querySelector("#lens-progress-notification")?.getAttribute("data-pending-count"),
    ).toBe("1");
    let finish!: (block: LensOutputBlock) => void;
    view.loadResponseBlock = () =>
      new Promise((resolve) => {
        finish = resolve;
      });
    await view.updateComplete;
    root.querySelector<HTMLButtonElement>(".lens-view-latest")!.click();
    await vi.waitFor(() => expect(finish).toBeDefined());
    view.model = { ...view.model!, lens: { ...view.model!.lens, operation_id: "next" } };
    view.responseHistory = history(3, "next");
    await view.updateComplete;
    finish({ type: "markdown", text: "obsolete" });
    await Promise.resolve();
    expect(root.querySelector(".lens-response-update")).toBeNull();
    expect(
      root.querySelector("#lens-progress-notification")?.getAttribute("data-pending-count"),
    ).toBe("0");
  });

  it("moves one notification host into and out of fullscreen while retaining approval draft and a single announcer", async () => {
    const view = await mountOverlay();
    view.model = {
      ...view.model!,
      lens: {
        ...view.model!.lens,
        session_controls: {
          instance_id: "controls",
          operation_id: "op",
          session_id: "session",
          agent_name: "Agent",
          active: true,
          config_revision: 1,
          modes: [],
          config_options: [],
          effective_mode: "safe",
          configured_mode: "safe",
          agent_default: "safe",
          interactions: [
            {
              id: "approval",
              sequence: 1,
              status: "pending",
              details: {
                kind: "form",
                message: "Approve this",
                schema: { type: "object", properties: { name: { type: "string", title: "Name" } } },
              },
            },
          ],
        },
      },
    };
    const response = history(1);
    const first = response.responses[0]!;
    view.responseHistory = {
      ...response,
      responses: [
        {
          ...first,
          blocks: [
            ...first.blocks,
            { type: "image", block_index: 1, mime_type: "image/png", byte_length: 4 },
          ],
        },
      ],
      media: [
        {
          kind: "image",
          id: responseBlockIdentity("op", "r1", 1),
          source: "data:image/png;base64,aA==",
          mimeType: "image/png",
        },
      ],
    };
    await view.updateComplete;
    const root = view.shadowRoot!;
    const output = root.querySelector<LensAgentOutput>("lens-agent-output")!;
    await output.updateComplete;
    const media = output.querySelector<LensOutputMedia>("lens-output-media")!;
    await media.updateComplete;
    const image = media.querySelector<HTMLImageElement>(".output-media-slide > img")!;
    Object.defineProperties(image, { naturalWidth: { value: 100 }, naturalHeight: { value: 100 } });
    image.dispatchEvent(new Event("load"));
    await media.updateComplete;
    const host = root.querySelector("#lens-progress-notification")!;
    const controls = host.querySelector("lens-session-controls")!;
    const input = controls.querySelector<HTMLInputElement>("input")!;
    input.value = "Preserve draft";
    let fullscreen: Element | null = null;
    const setFullscreen = (element: Element): void => {
      fullscreen = element;
    };
    Object.defineProperty(document, "fullscreenElement", {
      configurable: true,
      get: () => (fullscreen ? view : null),
    });
    Object.defineProperty(root, "fullscreenElement", { configurable: true, get: () => fullscreen });
    HTMLElement.prototype.requestFullscreen ??= async () => undefined;
    document.exitFullscreen ??= async () => undefined;
    vi.spyOn(HTMLElement.prototype, "requestFullscreen").mockImplementation(async function (
      this: HTMLElement,
    ) {
      setFullscreen(this);
      document.dispatchEvent(new Event("fullscreenchange"));
    });
    vi.spyOn(document, "exitFullscreen").mockImplementation(async () => {
      fullscreen = null;
      document.dispatchEvent(new Event("fullscreenchange"));
    });
    media.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    await vi.waitFor(() =>
      expect(media.querySelector(".output-media-expanded #lens-progress-notification")).toBe(host),
    );
    expect(host.querySelector("lens-session-controls")).toBe(controls);
    expect(host.querySelector("input")).toBe(input);
    expect(input.value).toBe("Preserve draft");
    view.responseHistory = {
      ...view.responseHistory!,
      responses: [...view.responseHistory!.responses, manifest(2)],
    };
    await view.updateComplete;
    expect(host.querySelector("input")).toBe(input);
    expect(root.querySelectorAll('[role="status"]')).toHaveLength(1);
    await media.exitFullscreen();
    await vi.waitFor(() =>
      expect(root.querySelector(".overlay-shell > #lens-progress-notification")).toBe(host),
    );
    expect(host.querySelector("input")).toBe(input);
    expect(input.value).toBe("Preserve draft");
    expect(root.querySelectorAll("#lens-progress-notification")).toHaveLength(1);
    expect(root.querySelectorAll('[role="status"]')).toHaveLength(1);
    view.model = { ...view.model!, lens: { ...view.model!.lens, session_controls: undefined } };
    await view.updateComplete;
    media.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    await vi.waitFor(() =>
      expect(media.querySelector(".output-media-expanded #lens-progress-notification")).toBe(host),
    );
    host.querySelector<HTMLButtonElement>(".lens-progress-dismiss")!.click();
    await view.updateComplete;
    const fullscreenToggle = host.querySelector<HTMLButtonElement>(".overlay-status-toggle")!;
    expect(fullscreenToggle.textContent).toContain("1 new response");
    expect(fullscreenToggle.getAttribute("aria-expanded")).toBe("false");
    fullscreenToggle.click();
    await view.updateComplete;
    expect(host.querySelector(".lens-view-latest")).not.toBeNull();
    view.model = { ...view.model!, lens: { ...view.model!.lens, operation_id: "text-only" } };
    view.responseHistory = history(1, "text-only");
    await view.updateComplete;
    await vi.waitFor(() =>
      expect(root.querySelector(".overlay-shell > #lens-progress-notification")).toBe(host),
    );
    expect(root.querySelectorAll("#lens-progress-notification")).toHaveLength(1);
    expect(root.querySelector(".lens-response-update")).toBeNull();
    view.sessionView = {
      phase: "ready",
      revision: 1,
      generation: "stored",
      session_id: "saved",
      agent: "codex",
    };
    view.responseHistory = {
      ...history(1, "history:stored"),
      media: [
        {
          kind: "image",
          id: responseBlockIdentity("history:stored", "r1", 1),
          source: "data:image/png;base64,aA==",
          mimeType: "image/png",
        },
      ],
    };
    await view.updateComplete;
    const storedOutput = root.querySelector<LensAgentOutput>("lens-agent-output")!;
    await storedOutput.updateComplete;
    const storedMedia = storedOutput.querySelector<LensOutputMedia>("lens-output-media")!;
    await storedMedia.updateComplete;
    const storedImage = storedMedia.querySelector<HTMLImageElement>(".output-media-slide > img")!;
    Object.defineProperties(storedImage, {
      naturalWidth: { value: 100 },
      naturalHeight: { value: 100 },
    });
    storedImage.dispatchEvent(new Event("load"));
    await storedMedia.updateComplete;
    storedMedia.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    await vi.waitFor(() =>
      expect(storedMedia.querySelector(".output-media-expanded #lens-progress-notification")).toBe(
        host,
      ),
    );
    view.model = {
      ...view.model!,
      lens: { ...view.model!.lens, operation_id: "unrelated-background-operation" },
    };
    await view.updateComplete;
    expect(storedMedia.querySelector(".output-media-expanded #lens-progress-notification")).toBe(
      host,
    );
    expect(fullscreen).toBe(storedMedia.querySelector(".output-media-expanded"));
    await storedMedia.exitFullscreen();
    delete (document as unknown as Record<string, unknown>).fullscreenElement;
  });
});

describe("Overlay committed response presentation", () => {
  it("presents successive responses automatically, retaining prior bodies and Interpretation focus", async () => {
    const view = await mountOverlay();
    const root = view.shadowRoot!;
    const panel = root.querySelector<HTMLElement>("#interpretation-panel")!;
    const first = root.querySelector("lens-response-block")!;
    visible(first);
    await vi.waitFor(() => expect(first.querySelector("h2")?.textContent).toBe("r1"));
    const paragraph = first.querySelector("p");
    panel.focus();
    for (const count of [2, 3]) {
      appendHistory(view, history(count));
      await view.updateComplete;
      const output = root.querySelector<LensAgentOutput>("lens-agent-output")!;
      await output.updateComplete;
      const latest = output.querySelectorAll("lens-response-block")[count - 1]!;
      visible(latest);
      await vi.waitFor(() => expect(latest.querySelector("h2")?.textContent).toBe(`r${count}`));
      expect(
        [...output.querySelectorAll("lens-response-block h2")].map((node) => node.textContent),
      ).toEqual(Array.from({ length: count }, (_, index) => `r${index + 1}`));
      expect(first.querySelector("p")).toBe(paragraph);
      expect(root.activeElement).toBe(panel);
      expect(root.querySelector(".lens-update-action")).toBeNull();
    }
    for (const stage of ["transforming", "failed"] as const) {
      view.model = { ...view.model!, lens: { ...view.model!.lens, stage } };
      await view.updateComplete;
      expect(root.querySelectorAll(".lens-response")).toHaveLength(3);
      expect(first.querySelector("p")).toBe(paragraph);
      expect(root.activeElement).toBe(panel);
    }
  });

  it("retains selected media and Interpretation focus across appends and deferred narrative growth", async () => {
    const view = await mountOverlay(2);
    const withMedia = (count: number): ResponseHistoryPresentation => {
      const presentation = history(count);
      return {
        ...presentation,
        responses: presentation.responses.map((response) => ({
          ...response,
          blocks: [
            ...response.blocks,
            { type: "image", block_index: 1, mime_type: "image/png", byte_length: 1 },
          ],
        })),
        media: presentation.responses.map((response) => ({
          kind: "image",
          id: responseBlockIdentity("op", response.id, 1),
          mimeType: "image/png",
          source: "data:image/png;base64,aA==",
        })),
      };
    };
    appendHistory(view, withMedia(2));
    await view.updateComplete;
    const root = view.shadowRoot!;
    const output = root.querySelector<LensAgentOutput>("lens-agent-output")!;
    await output.updateComplete;
    const media = output.querySelector<LensOutputMedia>("lens-output-media")!;
    await media.updateComplete;
    media.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await media.updateComplete;
    const selected = media.querySelector('.output-media-slide[aria-hidden="false"]')!;
    const panel = root.querySelector<HTMLElement>("#interpretation-panel")!;
    panel.focus();
    appendHistory(view, withMedia(3));
    await view.updateComplete;
    await output.updateComplete;
    await media.updateComplete;
    const latest = output.querySelectorAll("lens-response-block")[2]!;
    visible(latest);
    await vi.waitFor(() => expect(latest.querySelector("h2")?.textContent).toBe("r3"));
    resize(output.querySelector(".lens-output-narrative")!, 1400);
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    expect(output.querySelector("lens-output-media")).toBe(media);
    expect(media.querySelector('.output-media-slide[aria-hidden="false"]')).toBe(selected);
    expect(selected.getAttribute("aria-label")).toBe("Media 2 of 3");
    expect(root.activeElement).toBe(panel);
  });

  it("loads only visible narrative bodies and preserves old Markdown nodes while appending responses", async () => {
    load.mockClear();
    const element = await mount();
    const first = element.querySelector("lens-response-block")!;
    expect(load).not.toHaveBeenCalled();
    visible(first);
    await vi.waitFor(() => expect(first.querySelector("strong")?.textContent).toBe("paragraph"));
    const paragraph = first.querySelector("p");
    const markdown = first.querySelector("lens-markdown");
    element.history = history(3);
    await element.updateComplete;
    expect(element.querySelectorAll(".lens-response")).toHaveLength(3);
    expect(element.querySelector("lens-response-block")).toBe(first);
    expect(first.querySelector("p")).toBe(paragraph);
    expect(first.querySelector("lens-markdown")).toBe(markdown);
    expect(load).toHaveBeenCalledTimes(1);
    expect(
      [...element.querySelectorAll(".lens-response")].map((r) =>
        r.getAttribute("data-response-sequence"),
      ),
    ).toEqual(["1", "2", "3"]);
  });
  it("uses the shared chronological renderer for restored sessions without an inline document", async () => {
    const view = document.createElement("lens-overlay-view") as LensOverlayView;
    view.active = true;
    view.sessionView = {
      phase: "ready",
      revision: 1,
      generation: "restored",
      session_id: "saved",
      agent: "codex",
      interpretation: { responses: [] },
      conversation: { entries: [] },
    };
    view.responseHistory = history(3, "history:restored");
    view.loadResponseBlock = load;
    document.body.append(view);
    await view.updateComplete;
    const output = view.shadowRoot!.querySelector("lens-agent-output") as LensAgentOutput;
    await output.updateComplete;
    expect(output.history?.scopeId).toBe("history:restored");
    expect(
      [...output.querySelectorAll(".lens-response")].map((node) =>
        node.getAttribute("data-response-sequence"),
      ),
    ).toEqual(["1", "2", "3"]);
    const last = output.querySelectorAll("lens-response-block")[2]!;
    visible(last);
    await vi.waitFor(() => expect(last.querySelector("h2")?.textContent).toBe("r3"));
    expect(view.shadowRoot!.querySelector("lens-session-controls")).toBeNull();
    expect(view.shadowRoot!.querySelector("#conversation-tab")).not.toBeNull();
  });
  it("replaces a provisional first answer exactly once and starts an operation with separate DOM identity", async () => {
    const element = document.createElement("lens-agent-output") as LensAgentOutput;
    element.lens = {
      operation_id: "op",
      stage: "transforming",
      prompt_execution_revision: 1,
      response_history: { responses: [], retained_bytes: 0, capacity_reached: false },
      output_blocks: [{ type: "markdown", text: "Provisional" }],
    };
    document.body.append(element);
    await element.updateComplete;
    element.history = history(1);
    element.loadResponseBlock = load;
    await element.updateComplete;
    const first = element.querySelector("lens-response-block")!;
    visible(first);
    await vi.waitFor(() => expect(first.querySelector("strong")).not.toBeNull());
    expect(element.querySelectorAll(".lens-response")).toHaveLength(1);
    expect(element.textContent).not.toContain("Provisional");
    element.history = history(1, "another");
    await element.updateComplete;
    expect(element.querySelector("lens-response-block")).not.toBe(first);
  });
  it("retains measured placeholder height on tab detach and rejects old asynchronous bodies", async () => {
    const element = await mount();
    const cell = element.querySelector("lens-response-block") as LensResponseBlock;
    visible(cell);
    await vi.waitFor(() => expect(cell.querySelector("strong")).not.toBeNull());
    resize(cell, 420);
    element.remove();
    expect(cell.style.minHeight).toBe("420px");
    document.body.append(element);
    await element.updateComplete;
    expect(cell.style.minHeight).toBe("420px");
    visible(cell);
    await vi.waitFor(() => expect(cell.querySelector("strong")).not.toBeNull());
    let resolve!: (block: LensOutputBlock) => void;
    const other = document.createElement("lens-response-block") as LensResponseBlock;
    other.operationId = "old";
    other.responseId = "r";
    other.descriptor = { type: "markdown", block_index: 0, byte_length: 3 };
    other.loadBlock = () =>
      new Promise((r) => {
        resolve = r;
      });
    document.body.append(other);
    await other.updateComplete;
    visible(other);
    other.remove();
    resolve({ type: "markdown", text: "obsolete" });
    await Promise.resolve();
    expect(other.textContent).not.toContain("obsolete");
  });
  it("restores the visible response anchor when deferred content above it grows", async () => {
    const element = await mount(2);
    const output = element.querySelector<HTMLElement>(".lens-output")!;
    const [earlier, anchor] = element.querySelectorAll("lens-response-block");
    let top = 250;
    let anchorTop = 200;
    Object.defineProperties(output, {
      clientHeight: { get: () => 200 },
      scrollHeight: { get: () => 1800 },
      scrollTop: {
        get: () => top,
        set: (value: number) => {
          top = value;
        },
      },
    });
    vi.spyOn(output, "getBoundingClientRect").mockImplementation(() => ({ top: 0 }) as DOMRect);
    vi.spyOn(earlier!, "getBoundingClientRect").mockImplementation(
      () => ({ top: -top, bottom: 200 - top }) as DOMRect,
    );
    vi.spyOn(anchor!, "getBoundingClientRect").mockImplementation(
      () => ({ top: anchorTop - top, bottom: anchorTop + 200 - top }) as DOMRect,
    );
    output.dispatchEvent(new Event("scroll"));
    element.history = history(3);
    await element.updateComplete;
    anchorTop = 500;
    resize(element.querySelector(".lens-output-narrative")!, 1800);
    await vi.waitFor(() => expect(top).toBe(550));
    expect(anchor!.getBoundingClientRect().top).toBe(-50);
    expect(element.querySelectorAll("lens-response-block")[1]).toBe(anchor);
  });

  it("preserves reader offset on append and follows asynchronous content growth only from the end", async () => {
    const element = await mount();
    const output = element.querySelector(".lens-output") as HTMLElement;
    let height = 1000;
    let top = 100;
    Object.defineProperties(output, {
      clientHeight: { get: () => 200 },
      scrollHeight: { get: () => height },
      scrollTop: {
        get: () => top,
        set: (value: number) => {
          top = Math.min(value, height - 200);
        },
      },
    });
    output.dispatchEvent(new Event("scroll"));
    element.history = history(2);
    await element.updateComplete;
    height = 1400;
    resize(element.querySelector(".lens-output-narrative")!, 1400);
    await vi.waitFor(() => expect(top).toBe(100));
    top = 1200;
    output.dispatchEvent(new Event("scroll"));
    element.history = history(3);
    await element.updateComplete;
    height = 1800;
    resize(element.querySelector(".lens-output-narrative")!, 1800);
    await vi.waitFor(() => expect(top).toBe(1600));
    height = 2300;
    resize(element.querySelector(".lens-output-narrative")!, 2300);
    await vi.waitFor(() => expect(top).toBe(2100));
    top = 500;
    output.dispatchEvent(new Event("scroll"));
    height = 350;
    resize(element.querySelector(".lens-output-narrative")!, 350);
    await vi.waitFor(() => expect(top).toBe(150));
  });
});

it("retains each submitted response's text-only coverage across later complete responses and removes it in a new scope", async () => {
  const element = await mount(2);
  const first = element.history!.responses[0]!;
  element.history = {
    ...element.history!,
    responses: [
      {
        ...first,
        delivery: {
          mode: "text_only_partial",
          sources: [
            {
              source_id: "source-0",
              mode: "text_only_partial",
              omitted_media: [{ id: "image-0", reason: "image_not_supported" }],
            },
            {
              source_id: "source-1",
              mode: "unavailable",
              omitted_media: [{ id: "image-1", reason: "image_not_supported" }],
            },
          ],
        },
      },
      element.history!.responses[1]!,
    ],
  };
  await element.updateComplete;
  const coverage = element.querySelector<HTMLDetailsElement>(
    '[aria-label="Response 1 input coverage"]',
  )!;
  expect(coverage.open).toBe(false);
  expect(coverage.querySelector("summary")?.textContent?.replace(/\s+/g, " ")).toContain(
    "Response 1 · Agent input: PARTIAL",
  );
  expect(coverage.textContent).toContain("This response used text only");
  expect(coverage.textContent?.replace(/\s+/g, " ")).toContain(
    "Source 1: text only; images omitted",
  );
  expect(coverage.textContent?.replace(/\s+/g, " ")).toContain("Source 2: not interpreted");
  expect(element.querySelector('[aria-label="Response 2 input coverage"]')?.textContent).toContain(
    "UNKNOWN",
  );
  element.history = history(1, "next-operation");
  await element.updateComplete;
  expect(element.querySelector(".lens-delivery-notice")).toBeNull();
});

it("moves current unavailable-input detail to Diagnostics and clears stale coverage", async () => {
  const view = await mountOverlay(0);
  view.model = {
    ...view.model!,
    lens: {
      ...view.model!.lens,
      stage: "failed",
      projection: { revision: 1, digest: "captured" },
      delivery: {
        mode: "unavailable",
        projection: { revision: 1, digest: "delivered" },
        source_projection: { revision: 1, digest: "captured" },
        sources: [
          {
            source_id: "source-0",
            mode: "unavailable",
            omitted_media: [{ id: "image-0", reason: "image_not_supported" }],
          },
        ],
      },
    },
  };
  await view.updateComplete;
  expect(view.shadowRoot!.querySelector(".lens-delivery-notice")).toBeNull();
  expect(view.shadowRoot!.querySelector(".quality button:last-child")?.textContent?.trim()).toBe(
    "UNAVAILABLE",
  );
  const inputButton = view.shadowRoot!.querySelector<HTMLButtonElement>(
    ".quality button:last-child",
  )!;
  expect(inputButton.getAttribute("aria-controls")).toBe("diagnostics-panel");
  expect(inputButton.getAttribute("aria-label")).toContain(
    "prepared Agent input coverage unavailable; submission may still be pending",
  );
  expect(inputButton.title).toContain("submission may still be pending");
  expect(inputButton.title).toContain("Open Diagnostics");
  expect(inputButton.querySelector(".fa-robot")?.getAttribute("aria-hidden")).toBe("true");
  inputButton.click();
  await view.updateComplete;
  expect(view.shadowRoot!.querySelector("#diagnostics-panel")).not.toBeNull();
  expect(view.shadowRoot!.querySelector("#diagnostics-tab")?.getAttribute("aria-selected")).toBe(
    "true",
  );
  expect(view.shadowRoot!.activeElement).toBe(view.shadowRoot!.querySelector("#diagnostics-tab"));
  const captureButton = view.shadowRoot!.querySelector<HTMLButtonElement>(
    ".quality button:first-child",
  )!;
  expect(captureButton.getAttribute("aria-controls")).toBe("source-panel");
  expect(captureButton.title).toContain("Open Source");
  expect(captureButton.querySelector(".fa-camera")?.getAttribute("aria-hidden")).toBe("true");
  captureButton.click();
  await view.updateComplete;
  expect(view.shadowRoot!.querySelector("#source-tab")?.getAttribute("aria-selected")).toBe("true");
  expect(view.shadowRoot!.activeElement).toBe(view.shadowRoot!.querySelector("#source-tab"));
  view.sessionView = { revision: 1, phase: "ready", generation: "other-session" };
  await view.updateComplete;
  expect(view.shadowRoot!.querySelector(".quality")).toBeNull();
  view.sessionView = undefined;
  view.model = {
    ...view.model!,
    lens: { ...view.model!.lens, projection: { revision: 2, digest: "new-capture" } },
  };
  await view.updateComplete;
  expect(view.shadowRoot!.querySelector(".quality button:last-child")?.textContent?.trim()).toBe(
    "UNKNOWN",
  );
  view.model = { ...view.model!, lens: { ...view.model!.lens, delivery: undefined } };
  await view.updateComplete;
  expect(view.shadowRoot!.querySelector(".quality button:last-child")?.textContent?.trim()).toBe(
    "UNKNOWN",
  );
});

it("requires current projection evidence before the footer reports full Agent input", async () => {
  const view = await mountOverlay(0);
  const projection = { revision: 1, digest: "captured" };
  view.model = {
    ...view.model!,
    sourceMetadata: { has_input: true, quality: "partial", projection_has_loss: false },
    lens: {
      ...view.model!.lens,
      projection,
      delivery: {
        mode: "complete",
        source_projection: projection,
        projection: { revision: 1, digest: "delivered" },
        sources: [],
      },
    },
  };
  await view.updateComplete;
  const inputButton = view.shadowRoot!.querySelector<HTMLButtonElement>(
    ".quality button:last-child",
  )!;
  expect(inputButton.textContent?.trim()).toBe("FULL");
  expect(view.shadowRoot!.querySelector(".quality button:first-child")?.textContent?.trim()).toBe(
    "PARTIAL",
  );
  view.model = {
    ...view.model!,
    sourceMetadata: { ...view.model!.sourceMetadata!, projection_has_loss: true },
  };
  await view.updateComplete;
  expect(inputButton.textContent?.trim()).toBe("PARTIAL");
  view.model = {
    ...view.model!,
    sourceMetadata: { ...view.model!.sourceMetadata!, projection_has_loss: false },
    lens: {
      ...view.model!.lens,
      projection: { revision: 2, digest: "refreshed" },
    },
  };
  await view.updateComplete;
  expect(inputButton.textContent?.trim()).toBe("UNKNOWN");
});

it("does not report a complete projection before an input exists", async () => {
  const view = await mountOverlay(0);
  view.model = {
    ...view.model!,
    sourceMetadata: { has_input: false, quality: null, projection_has_loss: false },
  };
  await view.updateComplete;
  view.shadowRoot!.querySelector<HTMLButtonElement>(".quality button:last-child")!.click();
  await view.updateComplete;
  await view.shadowRoot!.querySelector<HTMLElement & { updateComplete: Promise<boolean> }>(
    "lens-extraction-diagnostics",
  )!.updateComplete;
  const projection = view.shadowRoot!.querySelector('[aria-labelledby="projection-loss-heading"]')!;
  expect(projection.textContent).toContain("Details unavailable");
  expect(projection.textContent).not.toContain(
    "No text, resource, or document content was omitted",
  );
  view.model = {
    ...view.model!,
    sourceMetadata: { has_input: true, quality: "full", projection_has_loss: false },
  };
  await view.updateComplete;
  await view.shadowRoot!.querySelector<HTMLElement & { updateComplete: Promise<boolean> }>(
    "lens-extraction-diagnostics",
  )!.updateComplete;
  expect(projection.textContent).toContain("Details unavailable");
});

it("keeps committed response coverage in history independently of a newer partial input", async () => {
  const view = await mountOverlay(1);
  const priorHistory: ResponseHistoryPresentation = {
    ...history(1),
    responses: [
      {
        ...manifest(1),
        delivery: {
          mode: "text_only_partial",
          sources: [{ source_id: "source-1", mode: "unavailable", omitted_media: [] }],
        },
      },
    ],
  };
  appendHistory(view, priorHistory);
  const delivery = {
    mode: "text_only_partial" as const,
    source_projection: { revision: 2, digest: "new-capture" },
    projection: { revision: 2, digest: "new-text" },
    sources: [
      {
        source_id: "source-0",
        mode: "text_only_partial" as const,
        omitted_media: [{ id: "media-0", reason: "image_not_supported" as const }],
      },
    ],
  };
  view.model = {
    ...view.model!,
    sourceMetadata: { has_input: true, quality: "full", projection_has_loss: false },
    lens: { ...view.model!.lens, projection: delivery.source_projection, delivery },
  };
  await view.updateComplete;
  await view.shadowRoot!.querySelector<LensAgentOutput>("lens-agent-output")!.updateComplete;
  const notices = [...view.shadowRoot!.querySelectorAll(".lens-delivery-notice")];
  expect(notices).toHaveLength(1);
  expect(notices[0]?.textContent).toContain("This response used text only");
  expect(notices[0]?.textContent?.replace(/\s+/g, " ")).toContain("Source 2: not interpreted");
  expect(notices[0]?.closest('[aria-label="Response 1 input coverage"]')).not.toBeNull();
  expect(view.shadowRoot!.querySelector(".quality button:last-child")?.textContent?.trim()).toBe(
    "PARTIAL",
  );
  view.shadowRoot!.querySelector<HTMLButtonElement>(".quality button:last-child")!.click();
  await view.updateComplete;
  await view.shadowRoot!.querySelector<HTMLElement & { updateComplete: Promise<boolean> }>(
    "lens-extraction-diagnostics",
  )!.updateComplete;
  const agentInput = view.shadowRoot!.querySelector('[aria-labelledby="agent-delivery-heading"]')!;
  expect(agentInput.textContent?.replace(/\s+/g, " ")).toContain("Images omitted for Agent 1");
  expect(agentInput.textContent?.replace(/\s+/g, " ")).toContain(
    "Source 1: Text only; 1 image omitted for Agent",
  );
  view.shadowRoot!.querySelector<HTMLButtonElement>("#interpretation-tab")!.click();
  await view.updateComplete;
  appendHistory(view, history(0));
  await view.updateComplete;
  await view.shadowRoot!.querySelector<LensAgentOutput>("lens-agent-output")!.updateComplete;
  expect(view.shadowRoot!.querySelectorAll(".lens-delivery-notice")).toHaveLength(0);
  expect(view.shadowRoot!.textContent).not.toContain("This response used text only");
});

it("preserves historical projection loss and missing evidence across later input changes", async () => {
  const element = await mount(3);
  element.history = {
    ...element.history!,
    responses: element.history!.responses.map((response, index) => ({
      ...response,
      delivery: {
        mode: "complete",
        sources: [],
        ...(index === 2 ? {} : { projection_has_loss: index === 0 }),
      },
    })),
  };
  await element.updateComplete;
  expect(element.querySelector('[aria-label="Response 1 input coverage"]')?.textContent).toContain(
    "PARTIAL",
  );
  expect(element.querySelector('[aria-label="Response 1 input coverage"]')?.textContent).toContain(
    "omitted when preparing",
  );
  expect(element.querySelector('[aria-label="Response 2 input coverage"]')).toBeNull();
  expect(element.querySelector('[aria-label="Response 3 input coverage"]')?.textContent).toContain(
    "UNKNOWN",
  );
  element.lens = { ...element.lens!, stage: "completed", delivery: undefined };
  await element.updateComplete;
  expect(element.querySelector('[aria-label="Response 1 input coverage"]')?.textContent).toContain(
    "PARTIAL",
  );
  expect(element.querySelector('[aria-label="Response 3 input coverage"]')?.textContent).toContain(
    "UNKNOWN",
  );
});
