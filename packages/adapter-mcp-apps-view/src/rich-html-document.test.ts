// @vitest-environment jsdom
// @vitest-environment-options {"jsdom":{"runScripts":"dangerously"}}
import { beforeAll, describe, expect, it } from "vitest";
import { prepareRichHtmlDocument as prepareDocument } from "./rich-html-document";
import { MATH_LIMITS } from "adapter-math-renderer";
import { createHtmlMathAssets, inlineHtmlMathCss } from "adapter-math-renderer/node";
let css = "";
beforeAll(async () => {
  const { manifest, sources } = await createHtmlMathAssets(process.cwd());
  css = inlineHtmlMathCss(manifest, sources);
});
const prepareRichHtmlDocument = (source: string) => prepareDocument(source, css);

function documentFixture(source: string, errors: string[] = []) {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  const owner = frame.contentWindow! as Window & { fixture?: number };
  owner.addEventListener("error", (event) => errors.push(event.message));
  owner.document.open();
  owner.document.write(source);
  owner.document.close();
  return { window: owner, close: () => frame.remove() };
}

describe("built-in rich HTML presentation derivative", () => {
  it("changes only eligible body text and preserves script/style/template/attribute math and unsafe TeX literally", () => {
    const script = String.raw`<script>const tex="\\(script\\)"; const text="<head>";</script>`;
    const source = String.raw`<!doctype html><head title=">"><style>.a::before{content:'\\(style\\)'}</style></head><body><pre>\(pre\)</pre><code>\(code\)</code><template>\(template\)</template><svg><text>\(svg\)</text></svg><span class="katex">\(existing\)</span><p title="\(attribute\)">\(x &lt; y\)</p><p>\(\unknown{unsafe}\)</p><p>$5 and $10 \(incomplete</p>${script}</body>`;
    const prepared = prepareRichHtmlDocument(source);
    const dom = documentFixture(prepared);
    try {
      expect(prepared).toContain(script);
      expect(prepared).toContain(String.raw`<template>\(template\)</template>`);
      expect(prepared).toContain(String.raw`title="\(attribute\)"`);
      expect(dom.window.document.querySelectorAll(".lens-html-math")).toHaveLength(1);
      expect(dom.window.document.querySelector("annotation")!.textContent).toBe("x < y");
      expect(dom.window.document.body.textContent).toContain(String.raw`\(\unknown{unsafe}\)`);
      expect(dom.window.document.body.textContent).toContain("$5 and $10");
    } finally {
      dom.close();
    }
  });
  it.each([
    String.raw`<!doctype html><p>\(x\)</p><script>window.fixture=73</script>`,
    String.raw`<html lang="en"><p>\(x\)</p><script>window.fixture=73</script></html>`,
    String.raw`<!doctype html><html><head title=">"><!-- <head> --><script>window.fixture=73</script></head><body><p>\(x\)</p></body></html>`,
  ])(
    "uses parser offsets for explicit and implied heads without rewriting author scripts",
    (source) => {
      const prepared = prepareRichHtmlDocument(source);
      const dom = documentFixture(prepared);
      try {
        expect(dom.window.fixture).toBe(73);
        expect(dom.window.document.querySelectorAll("script[data-lens-html-links]")).toHaveLength(
          1,
        );
        expect(dom.window.document.querySelectorAll(".lens-html-math")).toHaveLength(1);
      } finally {
        dom.close();
      }
    },
  );
  it("keeps expressions beyond the document count budget literal", () => {
    const prepared = prepareRichHtmlDocument(
      Array.from({ length: MATH_LIMITS.count + 1 }, () => String.raw`<p>\(x\)</p>`).join(""),
    );
    const dom = documentFixture(prepared);
    try {
      expect(dom.window.document.querySelectorAll(".lens-html-math")).toHaveLength(
        MATH_LIMITS.count,
      );
      expect(dom.window.document.querySelector("p:last-child")!.textContent).toBe(
        String.raw`\(x\)`,
      );
    } finally {
      dom.close();
    }
  });
  it("preserves UTF-8 source, node boundaries and foster-parented author markup", () => {
    const text = "\u65e5\u672c\u8a9e\u{1f642}";
    const script = `<script>window.fixture=73; const label="${text}";</script>`;
    const author = `<table data-original="yes">before<tr><td>${text} \\(x\\)</td></tr>after</table>`;
    const source = `<!doctype html><!-- <head> -->${script}<body>${author}<p><span>\\(split</span><span>\\)</span></p><img src="https://cdn.example/original.png"><a id="hash" href="#local">Local</a><a id="relative" href="/original">Relative</a></body>`;
    const prepared = prepareRichHtmlDocument(source);
    expect(prepared).toContain(script);
    expect(prepared).toContain('<img src="https://cdn.example/original.png">');
    expect(prepared).toContain("<span>\\(split</span><span>\\)</span>");
    const dom = documentFixture(prepared);
    try {
      expect(dom.window.fixture).toBe(73);
      expect(dom.window.document.querySelectorAll(".lens-html-math")).toHaveLength(1);
      expect(dom.window.document.querySelector("td")!.textContent).toContain(text);
      expect(dom.window.document.querySelector("table")!.getAttribute("data-original")).toBe("yes");
      expect(dom.window.document.querySelector("#hash")!.getAttribute("href")).toBe("#local");
      expect(dom.window.document.querySelector("#relative")!.getAttribute("href")).toBe(
        "/original",
      );
      expect(dom.window.document.body.textContent).toContain("before");
      expect(dom.window.document.body.textContent).toContain("after");
    } finally {
      dom.close();
    }
  });
  it("retains malformed and frameset documents without inventing math text nodes", () => {
    const source = String.raw`<!doctype html><html><head><title>\\(title\\)</title></head><frameset><frame src="https://example.com"></frameset></html>`;
    const prepared = prepareRichHtmlDocument(source);
    expect(prepared).toContain('<frameset><frame src="https://example.com"></frameset>');
    expect(prepared).not.toContain("data-lens-math");
    expect(prepared).toContain("data-lens-html-links");
  });
  it("adds no font payload to a document without math and rejects oversized raw input", () => {
    expect(prepareRichHtmlDocument("<p>plain</p>")).not.toContain("data-lens-math");
    expect(() => prepareRichHtmlDocument("a".repeat(512 * 1024 + 1))).toThrow("512 KiB");
  });
});
