// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import { externalMarkdownUrl, renderMarkdown, sanitizeMermaidSvg } from "./markdown";

describe("Agent Markdown rendering", () => {
  it("renders the GitHub Flavored Markdown constructs used by Agent responses", () => {
    const rendered = renderMarkdown(`
## Summary

| Item | State |
| --- | --- |
| ~~old~~ | **ready** |

- [x] Parsed
- [ ] Reviewed

https://example.com
`);

    const document = new DOMParser().parseFromString(rendered, "text/html");
    expect(document.querySelector("h2")?.textContent).toBe("Summary");
    expect(document.querySelector("table tbody td del")?.textContent).toBe("old");
    expect(document.querySelectorAll('input[type="checkbox"]')).toHaveLength(2);
    expect(document.querySelectorAll('input[type="checkbox"]:disabled')).toHaveLength(2);
    expect(document.querySelector('a[href="https://example.com"]')?.textContent).toBe(
      "https://example.com",
    );
  });

  it("removes executable markup and author-provided presentation", () => {
    const rendered = renderMarkdown(`
<script>globalThis.compromised = true</script>
<img src="x" onerror="globalThis.compromised = true">
<a href="javascript:alert(1)" style="position:fixed">unsafe</a>
<style>body { display: none }</style>
<svg onload="globalThis.compromised = true"></svg>
`);

    const document = new DOMParser().parseFromString(rendered, "text/html");
    expect(document.querySelector("script, style, svg")).toBeNull();
    expect(document.querySelector("img")?.hasAttribute("onerror")).toBe(false);
    expect(document.querySelector("a")?.hasAttribute("href")).toBe(false);
    expect(document.querySelector("a")?.hasAttribute("style")).toBe(false);
  });

  it("opens only explicit web and email links outside the Lens WebView", () => {
    expect(externalMarkdownUrl("https://example.com/path")).toBe("https://example.com/path");
    expect(externalMarkdownUrl("mailto:person@example.com")).toBe("mailto:person@example.com");
    expect(externalMarkdownUrl("javascript:alert(1)")).toBeUndefined();
    expect(externalMarkdownUrl("file:///etc/passwd")).toBeUndefined();
    expect(externalMarkdownUrl("/relative/path")).toBeUndefined();
  });

  it("normalizes malformed allowed Mermaid label markup through the same sanitizer", () => {
    const fragment = sanitizeMermaidSvg(`
      <svg xmlns="http://www.w3.org/2000/svg">
        <foreignObject width="200" height="40">
          <div xmlns="http://www.w3.org/1999/xhtml" style="display: table-cell; background: url(javascript:alert(1))">
            <span class="nodeLabel attacker"><p><b onclick="alert(2)">Unclosed label<script>alert(3)</script></p></span>
          </div>
        </foreignObject>
      </svg>
    `);
    const host = document.createElement("div");
    host.replaceChildren(fragment);

    expect(host.querySelector("b")?.textContent).toBe("Unclosed label");
    expect(host.querySelector("span")?.getAttribute("class")).toBe("nodeLabel");
    expect(host.querySelector("div")?.style.display).toBe("table-cell");
    expect(host.querySelector("div")?.style.background).toBe("");
    expect(host.querySelector("script, [onclick]")).toBeNull();
  });
});
