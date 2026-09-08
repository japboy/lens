// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { LensHtmlOutput } from "./lens-html-output";

afterEach(() => document.body.replaceChildren());

describe("HTML output component", () => {
  it("makes plain text content keyboard-focusable without intercepting scroll keys", async () => {
    const element = new LensHtmlOutput();
    element.resourceId = "plain-text";
    element.status = "ready";
    element.content = "<p>Keyboard-readable content without links.</p>";
    document.body.append(element);
    await element.updateComplete;
    const viewport = element.shadowRoot!.querySelector<HTMLElement>(".viewport")!;
    expect(viewport.tabIndex).toBe(0);
    expect(viewport.getAttribute("role")).toBe("region");
    expect(viewport.getAttribute("aria-label")).toBe("HTML content");
    viewport.focus();
    expect(element.shadowRoot!.activeElement).toBe(viewport);
    const event = new KeyboardEvent("keydown", {
      key: "PageDown",
      bubbles: true,
      composed: true,
      cancelable: true,
    });
    viewport.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it("keeps content identity and scroll position for unchanged resources", async () => {
    const element = new LensHtmlOutput();
    element.resourceId = "one";
    element.status = "ready";
    element.content = "<p>Content</p>";
    const ready = vi.fn<(event: Event) => void>();
    element.addEventListener("html-render-ready", ready);
    document.body.append(element);
    await element.updateComplete;
    const paragraph = element.shadowRoot!.querySelector("p");
    const viewport = element.shadowRoot!.querySelector<HTMLElement>(".viewport")!;
    viewport.scrollTop = 40;
    element.hidden = true;
    element.requestUpdate();
    await element.updateComplete;
    element.hidden = false;
    expect(element.shadowRoot!.querySelector("p")).toBe(paragraph);
    expect(viewport.scrollTop).toBe(40);
    expect(ready).toHaveBeenCalledTimes(1);
  });

  it("reports failures and sends link intent without navigating", async () => {
    const element = new LensHtmlOutput();
    element.resourceId = "one";
    element.status = "ready";
    element.content = '<a href="https://example.com">Read</a>';
    const link = vi.fn<(event: Event) => void>();
    element.addEventListener("html-open-link", link);
    document.body.append(element);
    await element.updateComplete;
    element.shadowRoot!.querySelector("a")!.click();
    expect((link.mock.calls[0]![0] as CustomEvent).detail).toEqual({ url: "https://example.com/" });
    const error = vi.fn<(event: Event) => void>();
    element.addEventListener("html-render-error", error);
    element.content = "x".repeat(512 * 1024 + 1);
    await element.updateComplete;
    await element.updateComplete;
    expect(error).toHaveBeenCalledTimes(1);
    expect(element.shadowRoot!.querySelector(".message")?.textContent).toContain("512 KiB");
    expect(element.shadowRoot!.querySelector("a")).toBeNull();
  });
});
