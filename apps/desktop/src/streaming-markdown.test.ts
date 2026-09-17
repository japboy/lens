// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { StreamingMarkdownElement } from "./streaming-markdown";
import { STREAMING_TAIL_LIMIT } from "./streaming-markdown-buffer";
import { protectStreamingMathSource } from "./streaming-math";

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
  it.each([
    "> ~~~tex\n> \\[a_i * b_j\\]\n> ~~~\n",
    "- ~~~tex\n  \\[a_i * b_j\\]\n  ~~~\n",
    "> - ~~~tex\n>   \\[a_i * b_j\\]\n>   ~~~\n",
  ])("preserves math-like source in nested code fences: %s", (source) => {
    expect(protectStreamingMathSource(source)).toBe(source);
    const element = mountedRenderer();
    for (let index = 1; index <= source.length; index++) {
      element.state = {
        operationId: "nested-fence",
        markdown: source.slice(0, index),
        phase: "streaming",
      };
      element.flush();
    }
    expect(element.textContent).not.toContain("&#");
    expect(element.textContent).toContain("a_i * b_j");
    expect(element.querySelector(".katex")).toBeNull();
    element.state = { ...element.state, phase: "settled" };
    expect(element.querySelector("pre code")?.textContent).toBe(String.raw`\[a_i * b_j\]` + "\n");
  });
  it.each([
    String.raw`Before \(a_i * b_j + \alpha\) after.`,
    "Before \\[a_i\n\n* b_j\n\\] after.",
    "Before $$a_i\n\n* b_j$$ after.",
  ])("preserves raw math through every character boundary: %s", (source) => {
    const element = mountedRenderer();
    for (let index = 1; index <= source.length; index++) {
      element.state = {
        operationId: "math-chunks",
        markdown: source.slice(0, index),
        phase: "streaming",
      };
      element.flush();
      expect(element.textContent?.replaceAll("▍", "")).toBe(source.slice(0, index));
      expect(element.querySelector(".katex, em, strong")).toBeNull();
    }
    element.state = { ...element.state, markdown: `${source}\n` };
    element.flush();
    expect(element.textContent?.replaceAll("▍", "")).toBe(source);
    expect(element.querySelector(".streaming-markdown-tail")).toBeNull();
  });

  it("typesets only when settled and retains committed nodes while receiving an incomplete formula", () => {
    const element = mountedRenderer();
    element.state = { operationId: "math-settle", markdown: "## Stable\n\n", phase: "streaming" };
    element.flush();
    const heading = element.querySelector("h2");
    const source = "## Stable\n\nEquation \\(a_i * b_j";
    element.state = { ...element.state, markdown: source };
    element.flush();
    expect(element.querySelector("h2")).toBe(heading);
    expect(element.querySelector(".streaming-markdown-tail")?.textContent).toBe(
      String.raw`Equation \(a_i * b_j`,
    );
    element.state = { ...element.state, markdown: `${source}\\)`, phase: "settled" };
    expect(element.querySelectorAll(".katex")).toHaveLength(1);
    expect(element.querySelector("math")).not.toBeNull();
    expect(element.querySelector(".streaming-markdown-tail")).toBeNull();
  });

  it("keeps math literal in tables and lists and preserves code and link destinations", () => {
    const element = mountedRenderer();
    element.state = {
      operationId: "math-structures",
      phase: "streaming",
      markdown:
        String.raw`| Formula | Value |
| --- | --- |
| \(a_i * b_j\) | 1 |

- \(c_i * d_j\)

Inline code: ` +
        "`\\(a_i * b_j\\)`" +
        String.raw` and [link](https://example.com/$$price$$).

` +
        "```tex\n\\[a_i * b_j\\]\n```\n",
    };
    element.flush();
    expect(element.querySelector("td")?.textContent).toBe(String.raw`\(a_i * b_j\)`);
    expect(element.querySelector("li")?.textContent).toContain(String.raw`\(c_i * d_j\)`);
    expect(element.querySelector("p code")?.textContent).toBe(String.raw`\(a_i * b_j\)`);
    expect(element.querySelector("pre code")?.textContent).toContain(String.raw`\[a_i * b_j\]`);
    expect(element.querySelector("a")?.getAttribute("href")).toBe("https://example.com/$$price$$");
  });

  it("keeps streamed escaped currency and completed-block/tail ordering", () => {
    const element = mountedRenderer();
    element.state = { operationId: "ordering", markdown: "First.\n\nTail", phase: "streaming" };
    element.flush();
    expect(element.textContent?.replaceAll("▍", "")).toBe("First.Tail");
    element.state = { ...element.state, markdown: "First.\n\nTail \\$5 and \\$10.\n\nLast" };
    element.flush();
    expect(element.textContent?.replaceAll("▍", "")).toBe("First.Tail $5 and $10.Last");
  });

  it("bounds an unfinished tail and resets literal fallback with operation identity", () => {
    const element = mountedRenderer();
    const source = "`" + "x".repeat(STREAMING_TAIL_LIMIT + 1);
    element.state = { operationId: "oversized", markdown: source, phase: "streaming" };
    element.flush();
    expect(element.querySelector('[data-streaming-fallback="capacity"]')?.textContent).toBe(source);
    element.state = { ...element.state, markdown: source + String.raw` \(x_i\)` };
    expect(element.textContent?.replaceAll("▍", "")).toBe(source + String.raw` \(x_i\)`);
    element.state = { operationId: "replacement", markdown: "**New**\n", phase: "streaming" };
    element.flush();
    expect(element.querySelector("strong")?.textContent).toBe("New");
    expect(element.querySelector(".streaming-markdown-tail")).toBeNull();
  });

  it("keeps a partial table header available across scheduled frames", async () => {
    const element = mountedRenderer();
    element.state = {
      operationId: "table-frames",
      markdown: "| Formula | Value |\n",
      phase: "streaming",
    };
    await new Promise((resolve) => window.setTimeout(resolve, 40));
    element.state = { ...element.state, markdown: element.state.markdown + "| --- | --- |\n" };
    await new Promise((resolve) => window.setTimeout(resolve, 40));
    element.state = {
      ...element.state,
      markdown: element.state.markdown + "| \\(a_i\\) | 1 |\n\nTail \\(",
    };
    await vi.waitFor(() =>
      expect(element.querySelector("td")?.textContent).toBe(String.raw`\(a_i\)`),
    );
    expect(element.lastElementChild?.classList.contains("streaming-cursor")).toBe(true);
    expect(element.querySelector(".streaming-markdown-tail")?.textContent).toBe(
      String.raw`Tail \(`,
    );
    expect(element.textContent?.indexOf(String.raw`\(a_i\)`)).toBeLessThan(
      element.textContent?.indexOf("Tail") ?? -1,
    );
  });

  it("does not encode math-like characters in indented code", () => {
    const element = mountedRenderer();
    element.state = {
      operationId: "indented-code",
      markdown: "    \\[a_i * b_j\\]\n\n",
      phase: "streaming",
    };
    element.flush();
    expect(element.querySelector("pre code")?.textContent).toContain(String.raw`\[a_i * b_j\]`);
    expect(element.textContent).not.toContain("&#");
  });

  it("keeps Mermaid source inert while the Agent response is streaming", () => {
    const element = mountedRenderer();
    element.state = {
      operationId: "operation-mermaid-stream",
      markdown:
        '```mermaid\nflowchart LR\n  A["<b>Bold</b> <em>emphasis</em> <code>code</code> plain"]\n```\n',
      phase: "streaming",
    };
    element.flush();

    expect(element.querySelector("pre > code")?.textContent).toContain(
      "<b>Bold</b> <em>emphasis</em> <code>code</code> plain",
    );
    expect(element.querySelector("figure.mermaid-diagram")).toBeNull();
    expect(element.getAttribute("aria-busy")).toBe("true");
  });

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

  it("stops following when media takes ownership of the opening viewport", async () => {
    const container = document.createElement("div");
    container.setAttribute("data-auto-scroll-container", "");
    Object.defineProperty(container, "scrollHeight", { value: 800 });
    const scrollTo = vi.fn<HTMLElement["scrollTo"]>();
    Object.defineProperty(container, "scrollTo", { value: scrollTo });
    const element = document.createElement("lens-markdown") as StreamingMarkdownElement;
    container.append(element);
    document.body.append(container);
    element.state = {
      operationId: "media-stream",
      markdown: "First paragraph.\n\n",
      phase: "streaming",
    };
    element.flush();
    await vi.waitFor(() => expect(scrollTo).toHaveBeenCalled());
    scrollTo.mockClear();
    element.state = {
      ...element.state,
      markdown: "First paragraph.\n\nA second paragraph.\n",
      scrollBehavior: "preserve",
    };
    element.flush();
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
    expect(element.textContent).toContain("A second paragraph.");
    expect(scrollTo).not.toHaveBeenCalled();
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
