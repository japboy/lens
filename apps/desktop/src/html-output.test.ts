// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { HTML_PREVIEW_CSP, MAX_HTML_BYTES, prepareHtmlPreview } from "./html-output";

const prepared = (content: string) =>
  new DOMParser().parseFromString(prepareHtmlPreview(content).document, "text/html");

describe("sandboxed static HTML preparation", () => {
  it("keeps escaped noscript content inert when serialized for a scripts-disabled frame", () => {
    const doc = prepared(
      '<body><noscript>&lt;meta http-equiv=refresh content="0;url=https://example.org/auto"&gt;</noscript></body>',
    );
    expect(doc.querySelector("noscript meta")).toBeNull();
    expect(doc.querySelector("noscript")?.textContent).toContain("<meta");
  });
  it("preserves document attributes, expressive CSS, SVG and native controls", () => {
    const css =
      ":root{--accent:oklch(70% .2 20)} @media(min-width:1px){body{display:grid;gap:calc(1rem + 2px)}} .card{position:fixed;inset:0;background:linear-gradient(red,blue)}";
    const doc = prepared(
      `<!doctype html><html lang="ja"><head><style>${css}</style></head><body class="card"><svg viewBox="0 0 10 10"><circle r="5"/></svg><details open><summary>More</summary>Content</details><input placeholder="Name"></body></html>`,
    );
    expect(doc.documentElement.lang).toBe("ja");
    expect(doc.body.className).toBe("card");
    expect(doc.querySelector("style")?.textContent).toBe(css);
    expect(doc.querySelector("svg circle")).not.toBeNull();
    expect(doc.querySelector("details")?.open).toBe(true);
    expect(doc.querySelector("input")?.placeholder).toBe("Name");
  });
  it("inserts the trusted CSP and removes navigation and embedded documents", () => {
    const doc = prepared(
      '<base href="https://evil.test"><meta http-equiv="refresh" content="0;url=https://evil.test"><meta http-equiv="Content-Security-Policy" content="script-src *"><script>alert(1)</script><iframe src="https://evil.test"></iframe><link rel="dns-prefetch" href="https://evil.test"><form action="https://evil.test"><button formaction="https://evil.test">Go</button></form>',
    );
    expect(doc.querySelectorAll("meta[http-equiv]")).toHaveLength(1);
    expect(doc.querySelector("meta[http-equiv]")?.getAttribute("content")).toBe(HTML_PREVIEW_CSP);
    expect(doc.querySelector("base")?.getAttribute("href")).toBe("about:srcdoc");
    expect(doc.querySelector("script,iframe,link,[action],[formaction]")).toBeNull();
    expect(doc.querySelector("form button")).not.toBeNull();
  });
  it("opens safe links through native popup requests and keeps fragments in the document", () => {
    const result = prepareHtmlPreview(
      '<a href="https://example.com">Reference</a><a href="#part">Jump</a><a href="javascript:alert(1)" onclick="alert(1)">Bad</a><svg><a href="https://example.com"><set attributeName="href" to="https://evil.test"/></a><circle><animate attributeName="r" values="1;5" dur="1s"/></circle></svg>',
    );
    const doc = new DOMParser().parseFromString(result.document, "text/html");
    expect(doc.querySelectorAll("a[href]")).toHaveLength(3);
    expect(doc.querySelector('a[href="#part"]')?.hasAttribute("target")).toBe(false);
    for (const link of doc.querySelectorAll('a[href="https://example.com/"]')) {
      expect(link.getAttribute("target")).toBe("_blank");
      expect(link.getAttribute("rel")).toBe("noopener noreferrer");
    }
    expect(doc.querySelector("[onclick],set")).toBeNull();
    expect(doc.querySelector("animate")).not.toBeNull();
  });
  it("accepts image-only and complex content, keeping the UTF-8 transport limit", () => {
    expect(prepared('<img src="data:image/png;base64,aA==">').querySelector("img")).not.toBeNull();
    expect(() => prepareHtmlPreview("<br>".repeat(5001))).not.toThrow();
    expect(() => prepareHtmlPreview("x".repeat(MAX_HTML_BYTES + 1))).toThrow("512 KiB");
    expect(() => prepareHtmlPreview("\u3042".repeat(MAX_HTML_BYTES / 3 + 1))).toThrow("512 KiB");
  });
});
