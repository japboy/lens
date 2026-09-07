// @vitest-environment jsdom

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { PresentedOutputImage } from "../output-media";
import type { LensState } from "../types";
import type { LensOutputMedia } from "./lens-output-media";
import type { LensAgentOutput } from "./lens-agent-output";

const images: readonly PresentedOutputImage[] = [
  { id: "result:image:0", source: "data:image/png;base64,aA==", mimeType: "image/png" },
  { id: "result:image:1", source: "data:image/jpeg;base64,dw==", mimeType: "image/jpeg" },
  { id: "result:image:2", source: "data:image/webp;base64,eA==", mimeType: "image/webp" },
];

beforeAll(async () => {
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  HTMLElement.prototype.scrollTo ??= () => undefined;
  HTMLElement.prototype.requestFullscreen ??= async () => undefined;
  document.exitFullscreen ??= async () => undefined;
  for (const target of [document, ShadowRoot.prototype]) {
    if (!("fullscreenElement" in target)) {
      Object.defineProperty(target, "fullscreenElement", {
        configurable: true,
        get: () => null,
      });
    }
  }
  await import("./lens-output-media");
  await import("./lens-agent-output");
});

let fullscreenElement: Element | null = null;
let resolveRequest: () => void;
let rejectRequest: (reason: Error) => void;
const requestFullscreen = vi.fn<HTMLElement["requestFullscreen"]>(function (this: HTMLElement) {
  return new Promise<void>((resolve, reject) => {
    resolveRequest = resolve;
    rejectRequest = reject;
  });
});
const exitFullscreen = vi.fn<Document["exitFullscreen"]>(async () => {
  setFullscreen(null);
});

function setFullscreen(element: Element | null): void {
  fullscreenElement = element;
  document.dispatchEvent(new Event("fullscreenchange"));
}

beforeEach(() => {
  fullscreenElement = null;
  requestFullscreen.mockClear();
  exitFullscreen.mockClear();
  vi.spyOn(document, "fullscreenElement", "get").mockImplementation(() => {
    const root = fullscreenElement?.getRootNode();
    return root instanceof ShadowRoot ? root.host : fullscreenElement;
  });
  vi.spyOn(ShadowRoot.prototype, "fullscreenElement", "get").mockImplementation(
    function (this: ShadowRoot) {
      return fullscreenElement?.getRootNode() === this ? fullscreenElement : null;
    },
  );
  vi.spyOn(HTMLElement.prototype, "requestFullscreen").mockImplementation(requestFullscreen);
  vi.spyOn(document, "exitFullscreen").mockImplementation(exitFullscreen);
});

afterEach(() => {
  fullscreenElement = null;
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

async function mount(media = images): Promise<LensOutputMedia> {
  const element = document.createElement("lens-output-media") as LensOutputMedia;
  element.media = media;
  document.body.append(element);
  await element.updateComplete;
  return element;
}

async function load(
  element: LensOutputMedia,
  index: number,
  width: number,
  height: number,
): Promise<void> {
  const image = element.querySelectorAll<HTMLImageElement>(".output-media-slide > img")[index]!;
  Object.defineProperties(image, {
    naturalWidth: { configurable: true, value: width },
    naturalHeight: { configurable: true, value: height },
  });
  image.dispatchEvent(new Event("load"));
  await element.updateComplete;
}

describe("Interpretation media interactions", () => {
  it("uses labeled Font Awesome controls and reports only loaded, known metadata", async () => {
    const element = await mount();
    const details = element.querySelector<HTMLButtonElement>('[aria-label="Media details"]')!;
    const expand = element.querySelector<HTMLButtonElement>('[aria-label="Expand media"]')!;
    expect(details.querySelector(".fa-circle-info")).not.toBeNull();
    expect(expand.querySelector(".fa-expand")).not.toBeNull();
    expect(details.textContent?.trim()).toBe("");
    expect(expand.textContent?.trim()).toBe("");
    expect(details.title).toBe("Details");
    expect(expand.disabled).toBe(true);
    details.click();
    await element.updateComplete;
    const panel = element.querySelector<HTMLElement>(".output-media-details")!;
    expect(panel.hidden).toBe(false);
    expect(panel.textContent).toContain("image/png");
    expect(panel.textContent).not.toContain("Intrinsic size");
    expect(panel.textContent).not.toContain("Title");
    await load(element, 0, 1200, 750);
    expect(panel.textContent).toContain("1200 × 750 CSS px");
    expect(panel.textContent).toContain("Landscape");
    expect(expand.disabled).toBe(false);
    details.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await element.updateComplete;
    expect(panel.hidden).toBe(true);
  });

  it("pages within finite boundaries and resets only for a changed media collection", async () => {
    const element = await mount();
    const next = element.querySelector<HTMLButtonElement>(".output-media-next")!;
    const previous = element.querySelector<HTMLButtonElement>(".output-media-previous")!;
    expect(previous.disabled).toBe(true);
    next.click();
    await element.updateComplete;
    expect(
      element.querySelector('[aria-hidden="false"].output-media-slide')?.getAttribute("aria-label"),
    ).toBe("Media 2 of 3");
    element.media = images.map((item) => ({ ...item }));
    await element.updateComplete;
    expect(
      element.querySelector('[aria-hidden="false"].output-media-slide')?.getAttribute("aria-label"),
    ).toBe("Media 2 of 3");
    next.focus();
    next.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await element.updateComplete;
    expect(next.disabled).toBe(true);
    expect(previous.disabled).toBe(false);
    expect(document.activeElement).toBe(previous);
    element.media = images.slice(0, 1);
    await element.updateComplete;
    expect(element.querySelector(".output-media-arrow")).toBeNull();
    expect(element.querySelector(".output-media-counter")).toBeNull();
    expect(
      element.querySelector('[aria-hidden="false"].output-media-slide')?.getAttribute("aria-label"),
    ).toBe("Media 1 of 1");
  });

  it("invalidates dimensions when the same snapshot position receives replacement data", async () => {
    const element = await mount(images.slice(0, 1));
    await load(element, 0, 750, 1200);
    const expand = element.querySelector<HTMLButtonElement>(".output-media-expand")!;
    expect(expand.disabled).toBe(false);
    element.media = [{ ...images[0]!, source: "data:image/png;base64,bmV3" }];
    await element.updateComplete;
    expect(expand.disabled).toBe(true);
    expect(element.querySelector(".output-media-details")?.textContent).not.toContain("750");
    element.querySelector(".output-media-slide > img")?.dispatchEvent(new Event("error"));
    await element.updateComplete;
    expect(element.querySelector(".output-media-state")?.textContent).toContain(
      "Unable to display this image.",
    );
    expect(expand.disabled).toBe(true);
  });

  it("requests fullscreen in the click and restores selection and focus on native exit", async () => {
    const element = await mount();
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    await load(element, 1, 750, 1200);
    const expand = element.querySelector<HTMLButtonElement>(".output-media-expand")!;
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    expect(expanded.tagName).toBe("DIV");
    expect(expanded.querySelector("img")?.getAttribute("src")).toBe(images[1]?.source);
    expand.focus();
    expand.click();
    expect(requestFullscreen).toHaveBeenCalledTimes(1);
    expect(requestFullscreen.mock.contexts[0]).toBe(expanded);
    await element.updateComplete;
    expect(expand.disabled).toBe(true);
    expand.click();
    expect(requestFullscreen).toHaveBeenCalledTimes(1);

    setFullscreen(expanded);
    resolveRequest();
    await element.updateComplete;
    await Promise.resolve();
    expect(document.activeElement).toBe(expanded.querySelector(".output-media-expanded-close"));

    // Escape, the browser's exit control, and OS exits all report fullscreenchange.
    setFullscreen(null);
    await element.updateComplete;
    await Promise.resolve();
    expect(document.activeElement).toBe(expand);
    expect(expand.disabled).toBe(false);
    expect(exitFullscreen).not.toHaveBeenCalled();
    expect(
      element.querySelector('[aria-hidden="false"].output-media-slide')?.getAttribute("aria-label"),
    ).toBe("Media 2 of 3");
  });

  it("reports a rejected request and allows retry without stale failure state", async () => {
    const element = await mount();
    await load(element, 0, 1200, 750);
    const expand = element.querySelector<HTMLButtonElement>(".output-media-expand")!;
    expand.click();
    rejectRequest(new Error("Fullscreen denied"));
    await Promise.resolve();
    await element.updateComplete;
    expect(element.querySelector('[role="alert"]')?.textContent).toMatch(/fullscreen/i);
    expect(expand.disabled).toBe(false);

    expand.click();
    expect(requestFullscreen).toHaveBeenCalledTimes(2);
    setFullscreen(element.querySelector(".output-media-expanded"));
    resolveRequest();
    await Promise.resolve();
    await element.updateComplete;
    expect(element.querySelector('[role="alert"]')).toBeNull();
  });

  it("closes owned fullscreen inside a shadow root through the standard exit API", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const shadow = host.attachShadow({ mode: "open" });
    const element = document.createElement("lens-output-media") as LensOutputMedia;
    element.media = images;
    shadow.append(element);
    await element.updateComplete;
    await load(element, 0, 1200, 750);
    const expand = element.querySelector<HTMLButtonElement>(".output-media-expand")!;
    expand.click();
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    setFullscreen(expanded);
    resolveRequest();
    await Promise.resolve();
    await element.updateComplete;
    expect(document.fullscreenElement).toBe(host);
    expect(shadow.fullscreenElement).toBe(expanded);
    expect(shadow.activeElement).toBe(expanded.querySelector(".output-media-expanded-close"));

    expanded.querySelector<HTMLButtonElement>(".output-media-expanded-close")!.click();
    expect(exitFullscreen).toHaveBeenCalledTimes(1);
    await Promise.resolve();
    await element.updateComplete;
    expect(shadow.activeElement).toBe(expand);
  });

  it("keeps fullscreen available for retry when exiting fails", async () => {
    const element = await mount();
    await load(element, 0, 1200, 750);
    const expand = element.querySelector<HTMLButtonElement>(".output-media-expand")!;
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    expand.click();
    setFullscreen(expanded);
    resolveRequest();
    await Promise.resolve();
    await element.updateComplete;

    const close = expanded.querySelector<HTMLButtonElement>(".output-media-expanded-close")!;
    let rejectExit!: (reason: Error) => void;
    exitFullscreen.mockImplementationOnce(
      () =>
        new Promise<void>((_resolve, reject) => {
          rejectExit = reject;
        }),
    );
    close.click();
    await element.updateComplete;
    expect(close.disabled).toBe(true);
    rejectExit(new Error("Exit denied"));
    await Promise.resolve();
    await element.updateComplete;
    expect(document.fullscreenElement).toBe(expanded);
    expect(close.disabled).toBe(false);
    expect(expand.disabled).toBe(true);
    expect(expanded.querySelector('[role="alert"]')?.textContent).toContain(
      "Unable to leave fullscreen",
    );

    close.click();
    await Promise.resolve();
    await element.updateComplete;
    expect(exitFullscreen).toHaveBeenCalledTimes(2);
    expect(document.fullscreenElement).toBeNull();
    expect(element.querySelector('[role="alert"]')).toBeNull();
    expect(document.activeElement).toBe(expand);
  });

  it("reports unavailable fullscreen without starting a request", async () => {
    const element = await mount();
    await load(element, 0, 1200, 750);
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    Object.defineProperty(expanded, "requestFullscreen", { value: undefined });
    const expand = element.querySelector<HTMLButtonElement>(".output-media-expand")!;
    expand.click();
    await element.updateComplete;
    expect(requestFullscreen).not.toHaveBeenCalled();
    expect(expand.disabled).toBe(false);
    expect(element.querySelector('[role="alert"]')?.textContent).toContain(
      "Fullscreen is unavailable",
    );
  });

  it("keeps keyboard focus in fullscreen and exits on Escape without paging", async () => {
    const element = await mount();
    await load(element, 0, 1200, 750);
    const expand = element.querySelector<HTMLButtonElement>(".output-media-expand")!;
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    expand.click();
    setFullscreen(expanded);
    resolveRequest();
    await Promise.resolve();
    await element.updateComplete;
    const close = expanded.querySelector<HTMLButtonElement>(".output-media-expanded-close")!;
    for (const shiftKey of [false, true]) {
      const tab = new KeyboardEvent("keydown", {
        key: "Tab",
        shiftKey,
        bubbles: true,
        cancelable: true,
      });
      close.dispatchEvent(tab);
      expect(tab.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(close);
    }
    close.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    expect(
      element.querySelector('[aria-hidden="false"].output-media-slide')?.getAttribute("aria-label"),
    ).toBe("Media 1 of 3");
    const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    close.dispatchEvent(escape);
    await Promise.resolve();
    await element.updateComplete;
    expect(escape.defaultPrevented).toBe(true);
    expect(exitFullscreen).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(expand);
  });

  it("preserves fullscreen for equivalent snapshots and updates to other media", async () => {
    const element = await mount();
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    await load(element, 1, 750, 1200);
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    setFullscreen(expanded);
    resolveRequest();
    await Promise.resolve();
    await element.updateComplete;
    for (const media of [
      images.map((item) => ({ ...item })),
      [images[0]!, images[1]!, { ...images[2]!, source: "data:image/webp;base64,bmV3" }],
    ]) {
      element.media = media;
      await element.updateComplete;
      expect(exitFullscreen).not.toHaveBeenCalled();
      expect(document.fullscreenElement).toBe(expanded);
      expect(expanded.querySelector("img")?.getAttribute("src")).toBe(images[1]!.source);
      expect(
        element
          .querySelector('[aria-hidden="false"].output-media-slide')
          ?.getAttribute("aria-label"),
      ).toBe("Media 2 of 3");
    }
  });

  it.each(["replacement", "removal", "disconnect"] as const)(
    "exits owned fullscreen on media %s",
    async (change) => {
      const element = await mount();
      await load(element, 0, 1200, 750);
      element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
      setFullscreen(element.querySelector(".output-media-expanded"));
      resolveRequest();
      await Promise.resolve();
      await element.updateComplete;

      if (change === "disconnect") element.remove();
      else {
        element.media =
          change === "removal" ? [] : [{ ...images[0]!, source: "data:image/png;base64,bmV3" }];
        await element.updateComplete;
      }
      expect(exitFullscreen).toHaveBeenCalledTimes(1);
    },
  );

  it("ignores a stale rejected request after media replacement", async () => {
    const element = await mount();
    await load(element, 0, 1200, 750);
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    element.media = [{ ...images[0]!, source: "data:image/png;base64,bmV3" }];
    await element.updateComplete;
    rejectRequest(new Error("Old request failed"));
    await Promise.resolve();
    await element.updateComplete;
    expect(element.querySelector('[role="alert"]')).toBeNull();
    expect(exitFullscreen).not.toHaveBeenCalled();
  });

  it("exits a request that enters fullscreen after the media was replaced", async () => {
    const element = await mount();
    await load(element, 0, 1200, 750);
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    element.media = [{ ...images[0]!, source: "data:image/png;base64,bmV3" }];
    await element.updateComplete;
    setFullscreen(expanded);
    resolveRequest();
    await Promise.resolve();
    await element.updateComplete;
    expect(exitFullscreen).toHaveBeenCalledTimes(1);
    expect(element.querySelector('[role="alert"]')).toBeNull();
  });

  it("does not exit another element's fullscreen when media changes or disconnects", async () => {
    const element = await mount();
    const other = document.createElement("div");
    document.body.append(other);
    setFullscreen(other);
    element.media = [];
    await element.updateComplete;
    element.remove();
    expect(exitFullscreen).not.toHaveBeenCalled();
  });
});

describe("media and narrative composition", () => {
  it("reveals the first image and disables Markdown following while media is present", async () => {
    const element = document.createElement("lens-agent-output") as LensAgentOutput;
    element.lens = {
      operation_id: "operation",
      stage: "transforming",
      output_blocks: [{ type: "markdown", text: "Initial prose" }],
    };
    document.body.append(element);
    await element.updateComplete;
    const output = element.querySelector<HTMLElement>(".lens-output")!;
    output.scrollTop = 500;
    element.lens = {
      ...element.lens,
      output_blocks: [
        { type: "markdown", text: "Initial prose" },
        { type: "image", mime_type: "image/png", data: "aA==" },
        { type: "markdown", text: "Continuing explanation" },
      ],
    };
    await element.updateComplete;
    expect(output.scrollTop).toBe(0);
    expect(output.firstElementChild?.tagName).toBe("LENS-OUTPUT-MEDIA");
    expect(element.querySelectorAll(".lens-output-narrative img")).toHaveLength(0);
    const markdown = element.querySelectorAll<HTMLElement & { state: { scrollBehavior: string } }>(
      "lens-markdown",
    );
    expect([...markdown].map((item) => item.state.scrollBehavior)).toEqual([
      "preserve",
      "preserve",
    ]);
    output.scrollTop = 240;
    element.lens = { ...element.lens, stage: "completed" };
    await element.updateComplete;
    expect(output.scrollTop).toBe(240);
    element.lens = {
      operation_id: "next",
      stage: "completed",
      output_blocks: [{ type: "markdown", text: "No media" }],
    } satisfies LensState;
    await element.updateComplete;
    expect(element.querySelector("lens-output-media")).toBeNull();
    expect(output.classList.contains("has-media")).toBe(false);
    expect(element.querySelector(".output-media-explanation")).toBeNull();
  });
});
