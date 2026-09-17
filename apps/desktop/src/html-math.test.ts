import { describe, expect, it } from "vitest";
import { parse, type DefaultTreeAdapterMap } from "parse5";
import { prepareHtmlPreview } from "./html-output";

type Node = DefaultTreeAdapterMap["node"];
function rendered(html: string) {
  const nodes: Node[] = [parse(html)];
  let math = 0;
  while (nodes.length) {
    const node = nodes.pop()!;
    if ("tagName" in node && node.tagName === "math") math++;
    if ("childNodes" in node) nodes.push(...node.childNodes);
  }
  return math;
}

describe("static HTML math preparation", () => {
  it("renders entity-decoded body text while preserving the sandbox CSP", () => {
    const result = prepareHtmlPreview(
      String.raw`<p>Price $5. &#92;(x^2&#92;)</p><div>$$\frac{1}{2}$$</div>`,
    );

    expect(rendered(result.document)).toBe(2);
    expect(result.document).toContain("script-src 'none'");
    expect(result.document).toContain('rel="stylesheet"');
    expect(result.document).toContain("lens-math://assets/assets/html-math/");
    expect(result.document).not.toContain("data:font/");
    expect(result.document).not.toContain("@font-face");
    expect(result.document).toContain("Price $5");
  });
  it("excludes attributes, code, inert templates, foreign trees and existing KaTeX", () => {
    const result = prepareHtmlPreview(
      String.raw`<p title="\(attribute\)">plain</p><pre>\(pre\)</pre><code>\(code\)</code><textarea>\(textarea\)</textarea><template>\(template\)</template><svg><text>\(svg\)</text></svg><math><mtext>\(math\)</mtext></math><span class="katex">\(existing\)</span><p>\(yes\)</p>`,
    );
    expect(rendered(result.document)).toBe(2);
    expect(result.document).toContain(String.raw`title="\(attribute\)"`);
    expect(result.document).toContain(String.raw`\(existing\)`);
  });
  it("never joins delimiters across element or comment boundaries", () => {
    const result = prepareHtmlPreview(String.raw`<p>\(<b>x</b>\)</p><p>\(x<!-- split -->\)</p>`);
    expect(rendered(result.document)).toBe(0);
  });
  it("preserves unfinished and invalid raw expressions and isolates macros", () => {
    const source = String.raw`<p>\(\gdef\foo{X}\foo\) \(\foo\)</p><p>\(unfinished</p><p>\(\unknowncommand{&lt;b&gt;}\)</p>`;
    const result = prepareHtmlPreview(source);
    expect(rendered(result.document)).toBe(1);
    expect(result.document).toContain(String.raw`\(\foo\)`);
    expect(result.document).toContain(String.raw`\(unfinished`);
    expect(result.document).toContain("&lt;b&gt;");
  });
  it("returns the entire baseline on output overflow without partial math or stylesheet", () => {
    const expression = `\\(${String.raw`\frac{x}{y}`.repeat(30)}\\)`;
    const source = `<h1>Original</h1><p>${Array.from({ length: 128 }, () => expression).join(" ")}</p>`;
    const result = prepareHtmlPreview(source);
    expect(result.notices).toContain(
      "Math rendering exceeded the preview output budget; original expressions are shown.",
    );
    expect(rendered(result.document)).toBe(0);
    expect(result.document).not.toContain("<link");
    expect(result.document).toContain(source);
  });
  it("retains source beyond formula budgets and preserves escaped opening delimiters", () => {
    const source = `<p>${String.raw`\\(escaped\)`} ${Array.from({ length: 129 }, () => String.raw`\(x\)`).join(" ")}</p>`;
    const result = prepareHtmlPreview(source);
    expect(rendered(result.document)).toBe(128);
    expect(result.document).toContain(String.raw`\\(escaped\)`);
    expect(result.document).toContain(String.raw`\(x\)`);
  });
  it("does not restore active HTML controls or trusted-only TeX commands", () => {
    const result = prepareHtmlPreview(
      String.raw`<script>alert(1)</script><img onerror="alert(1)" src="https://example.com/x"><p>\(\includegraphics{https://example.com/y}\) \(\href{javascript:alert(1)}{x}\) \(\htmlStyle{position:fixed}{x}\)</p>`,
    );
    expect(result.document).not.toContain("<script");
    expect(result.document).not.toContain("onerror");
    expect(result.document).not.toContain('href="javascript:');
    expect(result.document).not.toContain('style="position:fixed');
    expect(result.document).not.toContain('src="https://example.com/y');
  });
  it("adds no stylesheet for a document without eligible formulas", () => {
    const result = prepareHtmlPreview("<p>Plain $5 text</p>");
    expect(result.document).not.toContain("<link");
    expect(result.document).not.toContain("lens-html-math");
  });
  it.each(["xmp", "noembed", "noframes", "plaintext"])(
    "preserves formulas inside the %s raw-text context",
    (tag) => {
      const source = `<${tag}>\\(raw\\)</${tag}>`;
      const result = prepareHtmlPreview(source);
      expect(result.document).toContain(String.raw`\(raw\)`);
      expect(result.document).not.toContain("lens-html-math");
      expect(rendered(result.document)).toBe(0);
    },
  );
});
