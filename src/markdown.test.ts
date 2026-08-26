// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import { externalMarkdownUrl, renderMarkdown } from "./markdown";

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
});
