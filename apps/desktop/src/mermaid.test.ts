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
  it("renders only the finite HTML label vocabulary through strict configuration", async () => {
    const container = mountedMarkdown(`
\`\`\`mermaid
flowchart LR
  A --> B
\`\`\`
`);
    const initialize = vi.fn<(configuration: MermaidConfig) => void>();
    const render = vi.fn<(id: string, definition: string) => Promise<RenderResult>>(async () => ({
      diagramType: "flowchart-v2",
      svg: `<svg xmlns="http://www.w3.org/2000/svg" style="max-width: 200px" onload="alert(1)">
        <style>.node { fill: red; }</style>
        <script>alert(1)</script>
        <foreignObject x="1" y="2" width="200" height="40" class="attacker" style="background: red" onclick="alert(2)">
          <div xmlns="http://www.w3.org/1999/xhtml" class="labelBkg attacker" style="display: table-cell; white-space: nowrap; line-height: 1.5; max-width: 200px; width: 100000px; text-align: center; background: url(javascript:alert(3))" onclick="alert(4)">
            <span class="nodeLabel attacker" style="color: red" onmouseover="alert(5)">
              <p class="attacker" data-private="value">
                <b onclick="alert(6)">Bold</b>
                <strong>Strong</strong>
                <i>Italic</i>
                <em>Emphasis</em>
                <u>Underline</u>
                <s>Strike</s>
                <del>Deleted</del>
                <code>Code</code>
                H<sub>2</sub>
                x<sup>2</sup><br />
                Plain label
                <img src="https://attacker.invalid/image.png" />
                <a href="javascript:alert(7)">unsafe link</a>
                <script>alert(8)</script>
                <style>body { display: none; }</style>
                <svg><style>.mermaid-diagram { display: none; }</style><text>nested SVG</text></svg>
                <form><input name="payload" /></form>
                <mark>unsupported tag text</mark>
              </p>
            </span>
          </div>
        </foreignObject>
      </svg>`,
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
      htmlLabels: true,
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
    expect(configuration?.dompurifyConfig).toEqual({
      ALLOWED_TAGS: [
        "div",
        "span",
        "p",
        "b",
        "strong",
        "i",
        "em",
        "u",
        "s",
        "del",
        "code",
        "sub",
        "sup",
        "br",
      ],
      ALLOWED_ATTR: [],
      ALLOW_ARIA_ATTR: false,
      ALLOW_DATA_ATTR: false,
    });
    expect(container.querySelector("pre")).toBeNull();
    const figure = container.querySelector("figure.mermaid-diagram");
    const foreignObject = figure?.querySelector("foreignObject");
    const layout = foreignObject?.querySelector("div");
    const label = layout?.querySelector(":scope > span");
    expect(foreignObject).not.toBeNull();
    expect(foreignObject?.getAttributeNames().sort()).toEqual(["height", "width", "x", "y"]);
    expect(layout?.getAttribute("class")).toBe("labelBkg");
    expect(layout?.style.display).toBe("table-cell");
    expect(layout?.style.whiteSpace).toBe("nowrap");
    expect(layout?.style.lineHeight).toBe("1.5");
    expect(layout?.style.maxWidth).toBe("200px");
    expect(layout?.style.width).toBe("");
    expect(layout?.style.textAlign).toBe("center");
    expect(layout?.style.background).toBe("");
    expect(label?.getAttribute("class")).toBe("nodeLabel");
    expect(label?.hasAttribute("style")).toBe(false);
    expect(label?.querySelector("p")?.hasAttributes()).toBe(false);
    for (const tag of ["b", "strong", "i", "em", "u", "s", "del", "code", "sub", "sup", "br"]) {
      expect(label?.querySelector(tag), `${tag} should be retained`).not.toBeNull();
    }
    expect(label?.textContent).toContain("Plain label");
    expect(figure?.querySelector("style")?.textContent).toContain("fill: red");
    expect(figure?.querySelectorAll("style")).toHaveLength(1);
    expect(figure?.querySelector("svg")?.style.maxWidth).toBe("200px");
    expect(
      figure?.querySelector(
        "script, a, img, form, input, mark, foreignObject svg, [onload], [onclick], [onmouseover], [href], [src]",
      ),
    ).toBeNull();
  });

  it("keeps malformed Mermaid source visible and reports a finite error state", async () => {
    const container = mountedMarkdown("```mermaid\nnot a diagram\n```\n");
    const syntaxError = new Error("Parse error");
    const initialize = vi.fn<(configuration: MermaidConfig) => void>();

    const outcome = await renderMermaidCodeBlocks(container, {
      idPrefix: "invalid-diagram",
      isCurrent: () => true,
      loadEngine: async () => ({
        initialize,
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
    expect(initialize).toHaveBeenCalledWith(
      expect.objectContaining({
        darkMode: false,
        htmlLabels: true,
        securityLevel: "strict",
        theme: "default",
      }),
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
