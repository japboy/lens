// @vitest-environment jsdom
import "@lit-labs/ssr-client/lit-element-hydrate-support.js";
import { ReactiveElement } from "lit";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "../components/lens-about-view";
import type { LensAboutView } from "../components/lens-about-view";
import { installGeneratedPage } from "./generated-page.test-helper";
import { PageAttachment, type RegionModule } from "./page-attachment";

let regions: RegionModule[] = [];
class AttachmentHost extends ReactiveElement {
  readonly attachment = new PageAttachment(
    this,
    () => this.view,
    () => {},
    regions,
  );
  get view(): LensAboutView {
    return this.querySelector<LensAboutView>("lens-about-view")!;
  }
  protected createRenderRoot(): HTMLElement {
    return this;
  }
}
customElements.define("lens-about-page", AttachmentHost);

beforeEach(() => {
  document.body.replaceChildren();
  regions = [];
});

describe("independent region activation", () => {
  it("keeps the hydrated surface and another region usable when one module fails, then retries locally", async () => {
    const documents = vi
      .fn<() => Promise<void>>()
      .mockRejectedValueOnce(new Error("Offline"))
      .mockResolvedValue(undefined);
    const independent = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
    regions = [
      { name: "documents", ready: () => true, load: documents },
      { name: "independent", ready: () => true, load: independent },
    ];
    const page = installGeneratedPage("about") as AttachmentHost;
    const root = page.view.shadowRoot!;
    const select = root.querySelector("select")!;
    const region = root.querySelector("lens-license-document");
    await page.attachment.initialize();
    await vi.waitFor(() =>
      expect(root.querySelector("[data-region-error] [role=alert]")?.textContent).toContain(
        "Offline",
      ),
    );
    expect(independent).toHaveBeenCalledTimes(1);
    expect(page.attachment.stage).toBe("active");
    expect(page.view.shadowRoot).toBe(root);
    expect(root.querySelector("select")).toBe(select);
    expect(root.querySelector("lens-license-document")).toBe(region);
    root.querySelector<HTMLButtonElement>("[data-region-error] button")!.click();
    await vi.waitFor(() =>
      expect(root.querySelector("[data-region-error]")?.childNodes.length).toBe(0),
    );
    expect(documents).toHaveBeenCalledTimes(2);
    expect(independent).toHaveBeenCalledTimes(1);
    expect(root.querySelector("lens-license-document")).toBe(region);
  });

  it("waits for each region's own prerequisites and never rehydrates on reconnect", async () => {
    let ready = false;
    const load = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
    regions = [{ name: "documents", ready: () => ready, load }];
    const page = installGeneratedPage("about") as AttachmentHost;
    const root = page.view.shadowRoot!;
    await page.attachment.initialize();
    expect(load).not.toHaveBeenCalled();
    ready = true;
    page.requestUpdate();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    page.remove();
    document.body.append(page);
    await page.attachment.initialize();
    await page.updateComplete;
    expect(page.view.shadowRoot).toBe(root);
    expect(page.attachment.stage).toBe("active");
    expect(load).toHaveBeenCalledTimes(1);
  });
});
