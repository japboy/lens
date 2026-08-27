// @vitest-environment jsdom

import type { MermaidConfig, RenderResult } from "mermaid";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderMarkdown } from "./markdown";
import { renderMermaidCodeBlocks } from "./mermaid";

afterEach(() => {
  document.body.replaceChildren();
});

function mountedMarkdown(markdown: string): HTMLElement {
  const container = document.createElement("article");
  container.innerHTML = renderMarkdown(markdown);
  document.body.append(container);
  return container;
}

describe("settled Mermaid rendering", () => {
  it("replaces Mermaid code fences through strict, deterministic configuration", async () => {
    const container = mountedMarkdown(`
\`\`\`mermaid
flowchart LR
  A --> B
\`\`\`
`);
    const initialize = vi.fn<(configuration: MermaidConfig) => void>();
    const render = vi.fn<(id: string, definition: string) => Promise<RenderResult>>(async () => ({
      diagramType: "flowchart-v2",
      svg: '<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script><a href="https://example.com"><text>unsafe link</text></a><text>Safe diagram</text></svg>',
    }));

    const outcome = await renderMermaidCodeBlocks(container, {
      idPrefix: "test-diagram",
      isCurrent: () => true,
      loadEngine: async () => ({ initialize, render }),
      theme: "dark",
    });

    expect(outcome).toMatchObject({ errors: [], rendered: 1, status: "complete" });
    expect(render).toHaveBeenCalledWith("test-diagram-1", "flowchart LR\n  A --> B\n");
    const configuration = initialize.mock.calls[0]?.[0];
    expect(configuration).toMatchObject({
      darkMode: true,
      deterministicIDSeed: "test-diagram-1",
      deterministicIds: true,
      handDrawnSeed: 1,
      htmlLabels: false,
      logLevel: "fatal",
      maxEdges: 500,
      maxTextSize: 50_000,
      securityLevel: "strict",
      startOnLoad: false,
      suppressErrorRendering: true,
      theme: "dark",
    });
    expect(configuration?.secure).toEqual(
      expect.arrayContaining([
        "securityLevel",
        "maxTextSize",
        "maxEdges",
        "theme",
        "themeCSS",
        "dompurifyConfig",
      ]),
    );
    expect(container.querySelector("pre")).toBeNull();
    expect(container.querySelector("figure.mermaid-diagram svg")?.textContent).toContain(
      "Safe diagram",
    );
    expect(container.querySelector("script, a")).toBeNull();
  });

  it("keeps malformed Mermaid source visible and reports a finite error state", async () => {
    const container = mountedMarkdown("```mermaid\nnot a diagram\n```\n");
    const syntaxError = new Error("Parse error");

    const outcome = await renderMermaidCodeBlocks(container, {
      idPrefix: "invalid-diagram",
      isCurrent: () => true,
      loadEngine: async () => ({
        initialize: () => undefined,
        render: async () => Promise.reject(syntaxError),
      }),
      theme: "default",
    });

    expect(outcome).toEqual({ errors: [syntaxError], rendered: 0, status: "complete" });
    expect(container.querySelector("pre")?.dataset.mermaidState).toBe("error");
    expect(container.querySelector("code")?.textContent).toBe("not a diagram\n");
    expect(container.querySelector(".mermaid-error-message")?.textContent).toContain(
      "source is shown",
    );
  });

  it("does not commit a diagram after its Markdown state becomes stale", async () => {
    const container = mountedMarkdown("```mermaid\nflowchart LR\nA --> B\n```\n");
    let current = true;
    let finishRender: ((result: RenderResult) => void) | undefined;
    const pendingResult = new Promise<RenderResult>((resolve) => {
      finishRender = resolve;
    });

    const rendering = renderMermaidCodeBlocks(container, {
      idPrefix: "stale-diagram",
      isCurrent: () => current,
      loadEngine: async () => ({
        initialize: () => undefined,
        render: async () => pendingResult,
      }),
      theme: "default",
    });
    await vi.waitFor(() => {
      expect(container.querySelector("pre")?.dataset.mermaidState).toBe("rendering");
    });

    current = false;
    finishRender?.({ diagramType: "flowchart-v2", svg: "<svg></svg>" });
    const outcome = await rendering;

    expect(outcome).toMatchObject({ rendered: 0, status: "stale" });
    expect(container.querySelector("pre > code.language-mermaid")).not.toBeNull();
    expect(container.querySelector("figure")).toBeNull();
  });
});
