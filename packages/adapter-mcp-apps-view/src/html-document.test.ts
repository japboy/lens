import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import { describe, expect, it } from "vitest";
import { prepareHtmlDocument } from "./html-document";

const css = ".katex{font-family:serif}";
describe("shared HTML document presentation", () => {
  it("keeps local interactive behavior and fragment navigation after reopening a saved document", () => {
    const source = `<h1 id="section">Topic</h1><a href="#section">Jump</a><button onclick="document.querySelector('output').textContent='Second tab'">Tab</button><output>First tab</output><script>window.authorRuns=1</script><p>\\(x^2\\)</p>`;
    for (let index = 0; index < 2; index++) {
      const dom = new JSDOM(prepareHtmlDocument(source, "interactive", css), {
        url: "http://127.0.0.1/document/selected",
        runScripts: "dangerously",
      });
      dom.window.document.querySelector<HTMLButtonElement>("button")!.click();
      expect(dom.window.document.querySelector("output")!.textContent).toBe("Second tab");
      expect(dom.window.authorRuns).toBe(1);
      expect(dom.window.document.querySelector("math")).not.toBeNull();
      expect(dom.window.document.querySelector("a")!.getAttribute("href")).toBe("#section");
      dom.window.close();
    }
  });

  it("removes static author programs and navigation controls while preserving the same trusted helper and math", () => {
    const document = prepareHtmlDocument(
      `<html lang="ja"><style>body{color:red}</style><script>window.authorRuns=1</script><iframe src="https://bad.test"></iframe><form action="https://bad.test"><button onclick="window.authorRuns=2">Click</button></form><noscript>&lt;script&gt;window.authorRuns=3&lt;/script&gt;</noscript><a href="https://example.com" target="_blank">Reference</a><a href="#section">Jump</a><p>\\(x^2\\)</p></html>`,
      "static",
      css,
    );
    const dom = new JSDOM(document, {
      url: "http://127.0.0.1/document/selected",
      runScripts: "dangerously",
    });
    dom.window.document.querySelector<HTMLButtonElement>("button")!.click();
    expect(dom.window.authorRuns).toBeUndefined();
    expect(dom.window.document.querySelector("iframe,[onclick],[action]")).toBeNull();
    expect(dom.window.document.querySelectorAll("script")).toHaveLength(1);
    expect(dom.window.document.querySelector("script[data-lens-html-links]")!.textContent).toBe(
      readFileSync(new URL("./assets/rich-html-links.js", import.meta.url), "utf8"),
    );
    expect(dom.window.document.querySelector("math")).not.toBeNull();
    expect(dom.window.document.documentElement.lang).toBe("ja");
    expect(dom.window.document.querySelector('a[href="#section"]')).not.toBeNull();
    dom.window.close();
  });

  it("rejects over-budget UTF-8 input before either mode is parsed", () => {
    for (const mode of ["static", "interactive"] as const)
      expect(() => prepareHtmlDocument("\u65e5".repeat(180_000), mode, css)).toThrow("512 KiB");
  });

  it("allows valid source whose sanitized serialization expands beyond the source budget", () => {
    const source = `<p>${"<".repeat(140_000)}</p>`;
    const document = prepareHtmlDocument(source, "static", css);
    expect(new TextEncoder().encode(document).byteLength).toBeGreaterThan(512 * 1024);
    expect(document).toContain("&lt;&lt;&lt;");
  });
});
