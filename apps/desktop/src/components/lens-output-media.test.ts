// @vitest-environment jsdom

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { html } from "lit";
import type { PresentedOutputImage, PresentedOutputMedia } from "../output-media";
import type { LensState } from "../types";
import type { LensOutputMedia } from "./lens-output-media";
import type { LensAgentOutput } from "./lens-agent-output";

const images: readonly PresentedOutputImage[] = [
  {
    kind: "image",
    id: "result:image:0",
    source: "data:image/png;base64,aA==",
    mimeType: "image/png",
  },
  {
    kind: "image",
    id: "result:image:1",
    source: "data:image/jpeg;base64,dw==",
    mimeType: "image/jpeg",
  },
  {
    kind: "image",
    id: "result:image:2",
    source: "data:image/webp;base64,eA==",
    mimeType: "image/webp",
  },
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
  vi.spyOn(ShadowRoot.prototype, "fullscreenElement", "get").mockImplementation(function (
    this: ShadowRoot,
  ) {
    return fullscreenElement?.getRootNode() === this ? fullscreenElement : null;
  });
  vi.spyOn(HTMLElement.prototype, "requestFullscreen").mockImplementation(requestFullscreen);
  vi.spyOn(document, "exitFullscreen").mockImplementation(exitFullscreen);
});

afterEach(() => {
  fullscreenElement = null;
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

async function mount(media: readonly PresentedOutputMedia[] = images): Promise<LensOutputMedia> {
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
  const htmlMedia: PresentedOutputMedia = {
    kind: "html",
    id: "result:html:1",
    resourceId: "resource-1",
    mimeType: "text/html",
    uri: "urn:lens:test:html",
    byteLength: 100,
  };

  async function loadHtml(element: LensOutputMedia): Promise<HTMLIFrameElement> {
    element.htmlContent = {
      resourceId: "resource-1",
      status: "ready",
      content: '<h1>Readable result</h1><p><a href="https://example.com/">Reference</a></p>',
    };
    await element.updateComplete;
    const renderer = element.querySelector<HTMLIFrameElement>(".output-html-frame")!;
    renderer.dispatchEvent(new Event("load"));
    await element.updateComplete;
    return renderer;
  }

  it("renders the shared Notification only in fullscreen and keeps arrival passive with keyboard access", async () => {
    const element = await mount([images[0]!]);
    const action = vi.fn<() => void>();
    element.notificationContent = html`<div class="shared-notification" role="status">
      <button @click=${action}>View Latest</button>
    </div>`;
    await load(element, 0, 100, 100);
    expect(element.querySelector(".shared-notification")).toBeNull();
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    setFullscreen(expanded);
    resolveRequest();
    await element.updateComplete;
    const close = expanded.querySelector<HTMLButtonElement>(".output-media-expanded-close")!;
    expect(expanded.querySelectorAll('[role="status"]')).toHaveLength(1);
    element.media = images;
    element.notificationContent = html`<div class="shared-notification" role="status">
      <button @click=${action}>2 new responses</button>
    </div>`;
    await element.updateComplete;
    expect(fullscreenElement).toBe(expanded);
    expect(exitFullscreen).not.toHaveBeenCalled();
    expect(
      element.querySelector('.output-media-slide[aria-hidden="false"]')?.getAttribute("aria-label"),
    ).toBe("Media 1 of 3");
    close.focus();
    close.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }),
    );
    const latest = expanded.querySelector<HTMLButtonElement>(".shared-notification button")!;
    expect(document.activeElement).toBe(latest);
    latest.click();
    expect(action).toHaveBeenCalledOnce();
    latest.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true }),
    );
    expect(document.activeElement).toBe(close);
    await element.exitFullscreen();
    await element.updateComplete;
    expect(element.querySelector(".shared-notification")).toBeNull();
  });

  it("includes nested shadow form controls in fullscreen Tab order and respects their Escape handling", async () => {
    await import("./lens-select");
    const element = await mount([images[0]!]);
    const nested = document.createElement("div");
    nested.attachShadow({ mode: "open" }).innerHTML =
      '<lens-select></lens-select><button disabled>Unavailable</button><button tabindex="-1">Programmatic</button><button hidden>Hidden</button><button class="after">Continue</button>';
    const select = nested.shadowRoot!.querySelector(
      "lens-select",
    )! as import("./lens-select").LensSelect;
    select.options = [
      { value: "a", label: "Option A" },
      { value: "b", label: "Option B" },
    ];
    select.value = "a";
    element.notificationContent = nested;
    await load(element, 0, 100, 100);
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    const expanded = element.querySelector<HTMLElement>(".output-media-expanded")!;
    setFullscreen(expanded);
    resolveRequest();
    await element.updateComplete;
    await select.updateComplete;
    const close = expanded.querySelector<HTMLButtonElement>(".output-media-expanded-close")!;
    const combo = select.shadowRoot!.querySelector<HTMLButtonElement>("button")!;
    const after = nested.shadowRoot!.querySelector<HTMLButtonElement>(".after")!;
    const key = (target: HTMLElement, value: string, shiftKey = false) =>
      target.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: value,
          shiftKey,
          bubbles: true,
          composed: true,
          cancelable: true,
        }),
      );
    close.focus();
    key(close, "Tab");
    expect(select.shadowRoot!.activeElement).toBe(combo);
    key(combo, "ArrowDown");
    await select.updateComplete;
    key(combo, "Escape");
    await select.updateComplete;
    expect(combo.getAttribute("aria-expanded")).toBe("false");
    expect(exitFullscreen).not.toHaveBeenCalled();
    key(combo, "Tab");
    expect(nested.shadowRoot!.activeElement).toBe(after);
    key(after, "Tab", true);
    expect(select.shadowRoot!.activeElement).toBe(combo);
    key(combo, "Tab", true);
    expect(document.activeElement).toBe(close);
  });

  it("exits fullscreen before explicit navigation and completes only after the selected HTML loads", async () => {
    const element = await mount([images[0]!, htmlMedia]);
    await load(element, 0, 100, 100);
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    setFullscreen(element.querySelector(".output-media-expanded"));
    resolveRequest();
    await element.updateComplete;
    const state = vi.fn<(value: { selectedMediaId?: string; fullscreen: boolean }) => void>();
    element.addEventListener("lens-media-presentation", (event) => state(event.detail));
    let settled = false;
    const navigation = element.presentMedia(htmlMedia.id).then((value) => {
      settled = true;
      return value;
    });
    await Promise.resolve();
    await element.updateComplete;
    await Promise.resolve();
    await element.updateComplete;
    expect(exitFullscreen).toHaveBeenCalledOnce();
    expect(settled).toBe(false);
    await loadHtml(element);
    await expect(navigation).resolves.toBe(true);
    expect(state).toHaveBeenLastCalledWith({ selectedMediaId: htmlMedia.id, fullscreen: false });
    expect(element.querySelector("iframe")?.closest("[inert]")).toBeNull();
  });

  it("mirrors Notification inside HTML fullscreen without recreating its browsing context on arrival", async () => {
    const element = await mount([htmlMedia]);
    const frame = await loadHtml(element);
    element.notificationContent = html`<div class="shared-notification" role="status">
      One new response
    </div>`;
    await element.updateComplete;
    expect(element.querySelector(".shared-notification")).toBeNull();
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    const content = frame.parentElement!;
    setFullscreen(content);
    resolveRequest();
    await element.updateComplete;
    expect(content.querySelectorAll(".shared-notification")).toHaveLength(1);
    element.notificationContent = html`<div class="shared-notification" role="status">
      Two new responses
    </div>`;
    element.media = [htmlMedia, images[0]!];
    await element.updateComplete;
    expect(element.querySelector("iframe")).toBe(frame);
    expect(fullscreenElement).toBe(content);
    expect(exitFullscreen).not.toHaveBeenCalled();
  });

  it("does not navigate or acknowledge when fullscreen exit fails", async () => {
    const element = await mount();
    await load(element, 0, 100, 100);
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    setFullscreen(element.querySelector(".output-media-expanded"));
    resolveRequest();
    await element.updateComplete;
    exitFullscreen.mockRejectedValueOnce(new Error("denied"));
    await expect(element.presentMedia(images[1]!.id)).resolves.toBe(false);
    expect(
      element.querySelector('.output-media-slide[aria-hidden="false"]')?.getAttribute("aria-label"),
    ).toBe("Media 1 of 3");
    expect(fullscreenElement).not.toBeNull();
  });

  it("rejects superseded, failed, removed and disconnected navigation targets", async () => {
    const element = await mount();
    const first = element.presentMedia(images[1]!.id);
    await Promise.resolve();
    await element.updateComplete;
    const second = element.presentMedia(images[2]!.id);
    await expect(first).resolves.toBe(false);
    await element.updateComplete;
    element.mediaErrors = new Map([[images[2]!.id, "unavailable"]]);
    await element.updateComplete;
    await expect(second).resolves.toBe(false);
    const removed = element.presentMedia(images[0]!.id);
    await Promise.resolve();
    await element.updateComplete;
    element.media = [];
    await element.updateComplete;
    await expect(removed).resolves.toBe(false);
    element.media = images;
    await element.updateComplete;
    const disconnected = element.presentMedia(images[1]!.id);
    await Promise.resolve();
    await element.updateComplete;
    element.remove();
    await expect(disconnected).resolves.toBe(false);
  });

  it("mixes HTML and images without borrowing image dimensions or changing the controls", async () => {
    const element = await mount([images[0]!, htmlMedia]);
    expect(element.querySelector("iframe")).toBeNull();
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    const renderer = await loadHtml(element);
    expect(renderer.srcdoc).toContain("Readable result");
    expect(renderer.getAttribute("sandbox")).toBe("allow-popups");
    expect(renderer.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(renderer.title).toBe("HTML content");
    expect(renderer.closest(".output-media-slide")?.hasAttribute("inert")).toBe(false);
    expect(element.querySelector(".output-media-ambient")).toBeNull();
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(false);
    element.querySelector<HTMLButtonElement>(".output-media-details-toggle")!.click();
    await element.updateComplete;
    expect(element.querySelector(".output-media-details")?.textContent).toContain("text/html");
    expect(element.querySelector(".output-media-details")?.textContent).not.toContain(
      "Intrinsic size",
    );
    expect(element.querySelector(".output-media-overlay .fa-expand")).not.toBeNull();
  });

  it("remounts returning HTML under an interactive slide and keeps native-handled links", async () => {
    const element = await mount([images[0]!, htmlMedia]);
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    const renderer = await loadHtml(element);
    expect(renderer.closest(".output-media-slide")?.getAttribute("aria-hidden")).toBe("false");
    const preview = new DOMParser().parseFromString(renderer.srcdoc, "text/html");
    const anchor = preview.querySelector("a")!;
    expect(anchor.getAttribute("href")).toBe("https://example.com/");
    expect(anchor.getAttribute("target")).toBe("_blank");
    expect(anchor.getAttribute("rel")?.split(/\s+/)).toEqual(
      expect.arrayContaining(["noopener", "noreferrer"]),
    );
    expect(element.querySelector(".output-media-details button")).toBeNull();
    element.querySelector<HTMLButtonElement>(".output-media-previous")!.click();
    await element.updateComplete;
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    const replacement = element.querySelector(".output-html-frame");
    expect(replacement).not.toBe(renderer);
    expect(renderer.isConnected).toBe(false);
    expect(replacement?.closest("[inert]")).toBeNull();
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(true);
    renderer.dispatchEvent(new Event("load"));
    await element.updateComplete;
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(true);
    replacement!.dispatchEvent(new Event("load"));
    await element.updateComplete;
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(false);
  });

  it("fullscreens the existing HTML content inside its stable slide and does not trap Tab on the close control", async () => {
    const element = await mount([htmlMedia]);
    const renderer = await loadHtml(element);
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    await element.updateComplete;
    const slide = element.querySelector<HTMLElement>(".output-media-html-content")!;
    expect(requestFullscreen.mock.contexts[0]).toBe(slide);
    setFullscreen(slide);
    resolveRequest();
    await Promise.resolve();
    await element.updateComplete;
    const tab = new KeyboardEvent("keydown", {
      key: "Tab",
      bubbles: true,
      composed: true,
      cancelable: true,
    });
    element.querySelector<HTMLButtonElement>(".output-html-expanded-close")!.dispatchEvent(tab);
    expect(tab.defaultPrevented).toBe(false);
    expect(element.querySelector(".output-html-frame")).toBe(renderer);
    expect(element.querySelectorAll(".output-html-frame")).toHaveLength(1);
    element.querySelector<HTMLButtonElement>(".output-html-expanded-close")!.click();
    await Promise.resolve();
    await element.updateComplete;
    expect(exitFullscreen).toHaveBeenCalledOnce();
    expect(element.querySelector(".output-html-frame")).toBe(renderer);
  });

  it("keeps the final HTML flex slot and iframe when its inner content enters fullscreen", async () => {
    const element = await mount([...images, htmlMedia]);
    const rail = element.querySelector<HTMLElement>(".output-media-rail")!;
    Object.defineProperty(rail, "clientWidth", { configurable: true, value: 400 });
    for (let i = 0; i < 3; i++) {
      element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
      await element.updateComplete;
    }
    const renderer = await loadHtml(element);
    const frames = new Map<number, FrameRequestCallback>();
    let nextFrame = 0;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frames.set(++nextFrame, callback);
      return nextFrame;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation((id) => {
      frames.delete(id);
    });
    const flushFrame = async () => {
      const queued = [...frames.values()];
      frames.clear();
      queued.forEach((callback) => callback(0));
      await element.updateComplete;
    };
    rail.scrollLeft = 1200;
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    const slide = element.querySelector<HTMLElement>(".output-media-html-content")!;
    setFullscreen(slide);
    resolveRequest();
    await element.updateComplete;
    // Only the inner content leaves normal layout; the rail's four flex slots remain.
    expect(slide.parentElement?.classList.contains("output-media-html-slide")).toBe(true);
    expect(slide.parentElement?.parentElement).toBe(rail);
    expect(rail.querySelectorAll(":scope > .output-media-slide")).toHaveLength(4);
    expect(requestFullscreen.mock.contexts[0]).toBe(slide);
    expect(slide.contains(renderer)).toBe(true);
    rail.dispatchEvent(new Event("scroll"));
    setFullscreen(null);
    await element.updateComplete;
    await Promise.resolve();
    rail.dispatchEvent(new Event("scroll"));
    await flushFrame();
    await flushFrame();
    expect(rail.scrollLeft).toBe(1200);
    expect(element.querySelector(".output-media-html-slide")?.getAttribute("aria-hidden")).toBe(
      "false",
    );
    expect(element.querySelector(".output-html-frame")).toBe(renderer);
    // Real rail scrolling remains functional after exiting.
    rail.scrollLeft = 800;
    rail.dispatchEvent(new Event("scroll"));
    await flushFrame();
    expect(element.querySelector(".output-media-html-slide")?.getAttribute("aria-hidden")).toBe(
      "true",
    );
  });

  it("dismisses HTML details through a temporary backdrop without replacing the frame", async () => {
    const element = await mount([images[0]!, htmlMedia]);
    const toggle = element.querySelector<HTMLButtonElement>(".output-media-details-toggle")!;
    toggle.click();
    await element.updateComplete;
    expect(element.querySelector(".output-media-details-backdrop")).toBeNull();
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    const frame = await loadHtml(element);
    toggle.click();
    await element.updateComplete;
    const backdrop = element.querySelector<HTMLElement>(".output-media-details-backdrop")!;
    expect(backdrop).not.toBeNull();
    backdrop.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    await element.updateComplete;
    expect(element.querySelector(".output-media-details-backdrop")).toBeNull();
    expect(element.querySelector<HTMLElement>(".output-media-details")!.hidden).toBe(true);
    expect(element.querySelector(".output-html-frame")).toBe(frame);
  });

  it("keeps HTML failures local and disables expansion until safe content is ready", async () => {
    const element = await mount([htmlMedia]);
    element.htmlContent = {
      resourceId: "resource-1",
      status: "failed",
      message: "Resource expired",
    };
    await element.updateComplete;
    expect(element.querySelector(".output-media-state")!.textContent).toContain("Resource expired");
    expect(element.querySelector("iframe")).toBeNull();
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(true);
  });

  it("ignores old iframe loads and retains the current document across equivalent snapshots", async () => {
    const element = await mount([htmlMedia]);
    const oldFrame = await loadHtml(element);
    element.htmlContent = {
      resourceId: "resource-1",
      status: "ready",
      content: "<p>New content</p>",
    };
    await element.updateComplete;
    const frame = element.querySelector<HTMLIFrameElement>(".output-html-frame")!;
    expect(frame).not.toBe(oldFrame);
    oldFrame.dispatchEvent(new Event("load"));
    await element.updateComplete;
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(true);
    frame.dispatchEvent(new Event("load"));
    await element.updateComplete;
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(false);
    element.media = [{ ...htmlMedia }];
    await element.updateComplete;
    expect(element.querySelector(".output-html-frame")).toBe(frame);
  });

  it("prepares neighbors but mounts only selected HTML, retaining slide geometry and append DOM", async () => {
    const media = Array.from({ length: 30 }, (_, index) => ({
      ...htmlMedia,
      id: `response:${index}:html`,
      resourceId: "reused-resource",
    }));
    const element = document.createElement("lens-output-media") as LensOutputMedia;
    const requested: string[][] = [];
    element.addEventListener("lens-output-media-demand", (event) =>
      requested.push([...event.detail.mediaIds]),
    );
    element.media = media;
    element.htmlContents = new Map(
      media.map((item, index) => [
        item.id,
        {
          resourceId: item.resourceId,
          status: "ready" as const,
          content: `<h1>Response ${index}</h1>`,
        },
      ]),
    );
    document.body.append(element);
    await element.updateComplete;
    expect(requested).toEqual([[media[0]!.id, media[1]!.id]]);
    expect(element.querySelectorAll(".output-media-slide")).toHaveLength(30);
    const slides = [...element.querySelectorAll(".output-media-slide")];
    expect(element.querySelectorAll("iframe")).toHaveLength(1);
    const first = element.querySelector("iframe");
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    const selected = element.querySelector<HTMLIFrameElement>(
      '.output-media-slide[aria-hidden="false"] iframe',
    )!;
    expect(selected.srcdoc).toContain("Response 1");
    expect(element.querySelectorAll("iframe")).toHaveLength(1);
    expect(first?.isConnected).toBe(false);
    element.media = [...media, { ...htmlMedia, id: "appended" }];
    await element.updateComplete;
    expect(element.querySelector('.output-media-slide[aria-hidden="false"] iframe')).toBe(selected);
    const next = element.querySelector<HTMLButtonElement>(".output-media-next")!;
    next.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await element.updateComplete;
    expect(first?.isConnected).toBe(false);
    expect(element.querySelectorAll("iframe")).toHaveLength(0);
    expect([...element.querySelectorAll(".output-media-slide")].slice(0, 30)).toEqual(slides);
    expect(element.querySelector("[inert] .output-media-state")).toBeNull();
    expect(requested.at(-1)).toEqual([media[29]!.id, "appended"]);
  });

  it("synchronizes interaction with fractional slide positions beyond the initial responses", async () => {
    const media = Array.from({ length: 7 }, (_, index) => ({
      ...htmlMedia,
      id: `fractional:${index}`,
    }));
    const element = await mount(media);
    element.htmlContents = new Map(
      media.map((item) => [
        item.id,
        { resourceId: item.resourceId, status: "ready" as const, content: item.id },
      ]),
    );
    await element.updateComplete;
    const rail = element.querySelector<HTMLElement>(".output-media-rail")!;
    const slides = [...rail.children] as HTMLElement[];
    const width = 400.4;
    Object.defineProperty(rail, "clientWidth", { value: 400 });
    vi.spyOn(rail, "getBoundingClientRect").mockImplementation(
      () => ({ left: 0, width }) as DOMRect,
    );
    slides.forEach((slide, index) =>
      vi
        .spyOn(slide, "getBoundingClientRect")
        .mockImplementation(() => ({ left: index * width - rail.scrollLeft, width }) as DOMRect),
    );
    let frame!: FrameRequestCallback;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frame = callback;
      return 1;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
    for (const index of [4, 5, 6, 1]) {
      rail.scrollLeft = index * width;
      rail.dispatchEvent(new Event("scroll"));
      frame(0);
      await element.updateComplete;
      expect(slides[index]!.getAttribute("aria-hidden")).toBe("false");
      expect(slides[index]!.hasAttribute("inert")).toBe(false);
      expect(slides[index]!.querySelector("iframe")).not.toBeNull();
      expect(element.querySelectorAll("iframe")).toHaveLength(1);
    }
    const scroll = vi.spyOn(rail, "scrollTo");
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    const lastScroll = scroll.mock.calls.at(-1)!;
    expect((lastScroll[0] as ScrollToOptions).left).toBeCloseTo(2 * width);
    await element.updateComplete;
    expect([...rail.children]).toEqual(slides);
  });

  it("never borrows an equal resource ID from another response or the legacy HTML slot", async () => {
    const first = { ...htmlMedia, id: "response:1", resourceId: "shared" };
    const second = { ...htmlMedia, id: "response:2", resourceId: "shared" };
    const element = await mount([first, second]);
    element.htmlContent = { resourceId: "shared", status: "ready", content: "Legacy" };
    element.htmlContents = new Map([
      [first.id, { resourceId: "shared", status: "ready", content: "First" }],
    ]);
    await element.updateComplete;
    expect(element.querySelectorAll("iframe")).toHaveLength(1);
    const oldFrame = element.querySelector("iframe")!;
    element.media = [second];
    await element.updateComplete;
    oldFrame.dispatchEvent(new Event("load"));
    expect(element.querySelector("iframe")).toBeNull();
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(true);
    element.htmlContents = new Map([
      [second.id, { resourceId: "wrong", status: "ready", content: "Wrong" }],
    ]);
    await element.updateComplete;
    expect(element.querySelector("iframe")).toBeNull();
  });

  it("keeps a deferred image selected through body arrival and eviction and requests it again on reconnect", async () => {
    const deferred = { ...images[1]!, source: undefined };
    const element = await mount([images[0]!, deferred]);
    const requested: string[][] = [];
    element.addEventListener("lens-output-media-demand", (event) =>
      requested.push([...event.detail.mediaIds]),
    );
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    expect(element.querySelector('.output-media-slide[aria-hidden="false"] img')).toBeNull();
    element.mediaErrors = new Map([[deferred.id, "Image body is unavailable"]]);
    await element.updateComplete;
    expect(
      element.querySelector('.output-media-slide[aria-hidden="false"]')?.textContent,
    ).toContain("Image body is unavailable");
    expect(element.querySelector<HTMLButtonElement>(".output-media-expand")!.disabled).toBe(true);
    element.mediaErrors = new Map();
    element.media = [images[0]!, images[1]!];
    await element.updateComplete;
    expect(
      element.querySelector('.output-media-slide[aria-hidden="false"] img')?.getAttribute("src"),
    ).toBe(images[1]!.source);
    element.media = [images[0]!, deferred];
    await element.updateComplete;
    expect(
      element.querySelector('.output-media-slide[aria-hidden="false"]')?.getAttribute("aria-label"),
    ).toBe("Media 2 of 2");
    expect(element.querySelector('.output-media-slide[aria-hidden="false"] img')).toBeNull();
    element.remove();
    document.body.append(element);
    await element.updateComplete;
    expect(requested.at(-1)).toEqual([images[0]!.id, images[1]!.id]);
  });

  it("fullscreens the selected HTML response when earlier HTML slides coexist", async () => {
    const media = [0, 1].map((index) => ({ ...htmlMedia, id: `response:${index}` }));
    const element = await mount(media);
    element.htmlContents = new Map(
      media.map((item, index) => [
        item.id,
        {
          resourceId: item.resourceId,
          status: "ready" as const,
          content: `<h1>${index}</h1>`,
        },
      ]),
    );
    await element.updateComplete;
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    const selected = element.querySelector<HTMLIFrameElement>(
      '.output-media-slide[aria-hidden="false"] iframe',
    )!;
    selected.dispatchEvent(new Event("load"));
    await element.updateComplete;
    element.querySelector<HTMLButtonElement>(".output-media-expand")!.click();
    expect(requestFullscreen.mock.contexts[0]).toBe(selected.parentElement);
    setFullscreen(selected.parentElement);
    resolveRequest();
    await element.updateComplete;
    expect(element.querySelector('.output-media-slide[aria-hidden="false"] iframe')).toBe(selected);
  });

  it("uses labeled Font Awesome controls and reports only loaded, known metadata", async () => {
    const element = await mount();
    const details = element.querySelector<HTMLButtonElement>('[aria-label="Media Details"]')!;
    const expand = element.querySelector<HTMLButtonElement>('[aria-label="Expand Media"]')!;
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
      prompt_execution_revision: 1,
      output_blocks: [{ type: "markdown", text: "Initial prose" }],
    };
    document.body.append(element);
    await element.updateComplete;
    const output = element.querySelector<HTMLElement>(".lens-output")!;
    output.scrollTop = 500;
    element.lens = {
      ...element.lens,
      prompt_execution_revision: 1,
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
      prompt_execution_revision: 1,
      output_blocks: [{ type: "markdown", text: "No media" }],
    } satisfies LensState;
    await element.updateComplete;
    expect(element.querySelector("lens-output-media")).toBeNull();
    expect(output.classList.contains("has-media")).toBe(false);
    expect(element.querySelector(".output-media-explanation")).toBeNull();
  });
});
