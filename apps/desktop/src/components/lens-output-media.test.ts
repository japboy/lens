// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it } from "vitest";
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
  HTMLDialogElement.prototype.showModal ??= function () {
    this.open = true;
  };
  HTMLDialogElement.prototype.close ??= function () {
    if (!this.open) return;
    this.open = false;
    this.dispatchEvent(new Event("close"));
  };
  await import("./lens-output-media");
  await import("./lens-agent-output");
});

afterEach(() => {
  document.body.replaceChildren();
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

  it("closes expanded media without changing selection or the native close action", async () => {
    const element = await mount();
    element.querySelector<HTMLButtonElement>(".output-media-next")!.click();
    await element.updateComplete;
    await load(element, 1, 750, 1200);
    const expand = element.querySelector<HTMLButtonElement>(".output-media-expand")!;
    expand.click();
    await element.updateComplete;
    const dialog = element.querySelector<HTMLDialogElement>("dialog")!;
    expect(dialog.open).toBe(true);
    expect(dialog.querySelector("img")?.getAttribute("src")).toBe(images[1]?.source);
    dialog.dispatchEvent(new Event("cancel", { cancelable: true }));
    await element.updateComplete;
    expect(dialog.open).toBe(false);
    expect(
      element.querySelector('[aria-hidden="false"].output-media-slide')?.getAttribute("aria-label"),
    ).toBe("Media 2 of 3");
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
