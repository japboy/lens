// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { StreamingMarkdownElement } from "./streaming-markdown";

beforeAll(() => {
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  globalThis.requestAnimationFrame ??= (callback: FrameRequestCallback) =>
    window.setTimeout(() => callback(performance.now()), 0);
  globalThis.cancelAnimationFrame ??= (handle: number) => window.clearTimeout(handle);
  HTMLElement.prototype.scrollTo ??= () => undefined;
  if (!customElements.get("lens-markdown")) {
    customElements.define("lens-markdown", StreamingMarkdownElement);
  }
});

afterEach(() => {
  document.body.replaceChildren();
});

function mountedRenderer(): StreamingMarkdownElement {
  const element = document.createElement("lens-markdown") as StreamingMarkdownElement;
  document.body.append(element);
  return element;
}

describe("streaming Agent Markdown", () => {
  it("appends ACP deltas without replacing already committed blocks", () => {
    const element = mountedRenderer();
    element.state = {
      operationId: "operation-one",
      markdown: "## Stable heading\n\nFirst paragraph.\n\n",
      phase: "streaming",
    };
    element.flush();
    const heading = element.querySelector("h2");
    expect(heading?.textContent).toBe("Stable heading");

    element.state = {
      operationId: "operation-one",
      markdown: "## Stable heading\n\nFirst paragraph.\n\nSecond **streaming** paragraph.\n",
      phase: "streaming",
    };
    element.flush();

    expect(element.querySelector("h2")).toBe(heading);
    expect(element.textContent).toContain("Second streaming paragraph.");
    expect(element.querySelector("strong")?.textContent).toBe("streaming");
    expect(element.getAttribute("aria-busy")).toBe("true");
  });

  it("settles once through the sanitized GFM renderer", () => {
    const element = mountedRenderer();
    element.state = {
      operationId: "operation-two",
      markdown: `
| Item | State |
| --- | --- |
| ~~old~~ | **ready** |

- [x] Complete

<script>globalThis.compromised = true</script>
`,
      phase: "settled",
    };

    expect(element.querySelector("table tbody td del")?.textContent).toBe("old");
    expect(element.querySelector('input[type="checkbox"]:disabled')).not.toBeNull();
    expect(element.querySelector("script")).toBeNull();
    expect(element.getAttribute("aria-busy")).toBe("false");
    expect(element.hasAttribute("data-streaming")).toBe(false);
  });

  it("restarts the append-only renderer when an operation identity changes", () => {
    const element = mountedRenderer();
    element.state = {
      operationId: "old-operation",
      markdown: "Old response.\n",
      phase: "streaming",
    };
    element.flush();

    element.state = {
      operationId: "new-operation",
      markdown: "New response.\n",
      phase: "streaming",
    };
    element.flush();

    expect(element.textContent).toContain("New response.");
    expect(element.textContent).not.toContain("Old response.");
  });

  it("auto-scrolls the declared ancestor scroll container", async () => {
    const container = document.createElement("div");
    container.setAttribute("data-auto-scroll-container", "");
    Object.defineProperty(container, "scrollHeight", { configurable: true, value: 480 });
    const scrollTo = vi.fn<(options: ScrollToOptions) => void>();
    Object.defineProperty(container, "scrollTo", { configurable: true, value: scrollTo });

    const element = document.createElement("lens-markdown") as StreamingMarkdownElement;
    container.append(element);
    document.body.append(container);

    element.state = {
      operationId: "operation-scroll",
      markdown: "Streaming content.\n",
      phase: "streaming",
    };
    element.flush();

    await vi.waitFor(() => {
      expect(scrollTo).toHaveBeenCalledWith({ top: 480, behavior: "auto" });
    });
  });
});
