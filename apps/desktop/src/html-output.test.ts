// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { MAX_HTML_BYTES, renderStaticHtml } from "./html-output";

describe("static HTML profile", () => {
  it("drops a long invalid declaration without whitespace backtracking", () => {
    const invalid = `color:${" ".repeat(MAX_HTML_BYTES - 256)}:red`;
    const fragment = renderStaticHtml(`<p style="${invalid};color:blue">Readable</p>`);
    expect(fragment.querySelector("p")?.getAttribute("style")).toBe("color:blue");
  });

  it("bounds selector length and descendant matching complexity", () => {
    const allowed = Array.from({ length: 16 }, () => ".a").join(" ");
    const tooDeep = `${allowed} .a`;
    const maximum = `.${"a".repeat(511)}`;
    const tooLong = `${maximum}a`;
    const fragment = renderStaticHtml(
      `<style>${allowed}{color:blue}${tooDeep}{color:red}${maximum}{color:green}${tooLong}{color:purple}</style><p>Readable</p>`,
    );
    expect(fragment.querySelector("style")?.textContent).toBe(
      `.content ${allowed}{color:blue}\n.content ${maximum}{color:green}`,
    );
  });
  it("discards near-limit malformed stylesheets without suffix rescanning", () => {
    const text = "a".repeat(MAX_HTML_BYTES - 128);
    const fragment = renderStaticHtml(`<style>${text}</style><p>Readable</p>`);
    expect(fragment.querySelector("style")).toBeNull();
    expect(fragment.querySelector("p")?.textContent).toBe("Readable");
  });

  it("rejects incomplete and nested rules but preserves a valid rule sequence", () => {
    for (const css of [
      ".card{color:red",
      ".card{color:red}}",
      ".card{.child{color:red}}",
      ".card{color:red}trailing",
    ]) {
      expect(
        renderStaticHtml(`<style>${css}</style><p>Readable</p>`).querySelector("style"),
      ).toBeNull();
    }
    expect(
      renderStaticHtml(
        "<style> .a{color:red} .b{color:blue} </style><p>Readable</p>",
      ).querySelector("style")?.textContent,
    ).toBe(".content .a{color:red}\n.content .b{color:blue}");
  });

  it("ignores inherited property names while keeping allowed declarations", () => {
    const fragment = renderStaticHtml(
      '<style>.card{constructor:bad;color:red}</style><p class="card" style="constructor:bad;color:blue">Readable</p>',
    );
    expect(fragment.querySelector("style")?.textContent).toBe(".content .card{color:red}");
    expect(fragment.querySelector("p")?.getAttribute("style")).toBe("color:blue");
  });
  it("preserves full document text, tables and finite card styles", () => {
    const fragment = renderStaticHtml(
      '<!doctype html><html><head><style>.card {display:grid;gap:12px;background-color:#fff;grid-template-columns:1fr 2fr}</style></head><body><section class="card"><h1>Result</h1><table><tr><td style="padding:8px;color:blue">Interpretation</td></tr></table></section></body></html>',
    );
    expect(fragment.querySelector("h1")?.textContent).toBe("Result");
    expect(fragment.querySelector("td")?.getAttribute("style")).toBe("padding:8px;color:blue");
    expect(fragment.querySelector("style")?.textContent).toContain(".content .card{display:grid");
  });

  it("removes executable, embedded, custom and network loading elements", () => {
    const fragment = renderStaticHtml(
      '<script>alert(1)</script><img src="https://evil.test"><iframe src="https://evil.test"></iframe><link rel="stylesheet" href="https://evil.test"><svg onload="alert(1)"></svg><math><mtext>bad</mtext></math><form><input autofocus></form><x-evil>text</x-evil><p onclick="alert(1)" style="background-image:url(https://evil.test)">safe</p><a href="javascript:alert(1)">bad</a>',
    );
    expect(
      fragment.querySelector(
        "script,img,iframe,link,svg,math,form,input,x-evil,[onclick],[style],[href]",
      ),
    ).toBeNull();
    expect(fragment.textContent).toContain("safe");
  });

  it("rejects CSS escapes, host selectors, positioning and resource functions", () => {
    const fragment = renderStaticHtml(
      '<style>:host {display:none}.viewport{display:none}.card{position:fixed;inset:0;background:url(https://evil.test);color:red;--x:blue;width:var(--x)}</style><div class="card" style="color:blue;position:absolute">Card</div>',
    );
    const css = fragment.querySelector("style")?.textContent ?? "";
    expect(css).toBe(".content .viewport{display:none}\n.content .card{color:red}");
    expect(fragment.querySelector(".card")?.getAttribute("style")).toBe("color:blue");
    expect(
      renderStaticHtml("<style>.card{c\\6flor:red}</style><p>ok</p>").querySelector("style"),
    ).toBeNull();
  });

  it("bounds input bytes and DOM complexity", () => {
    expect(() => renderStaticHtml("x".repeat(MAX_HTML_BYTES + 1))).toThrow("512 KiB");
    expect(() => renderStaticHtml("<div>".repeat(70) + "x" + "</div>".repeat(70))).toThrow(
      "complexity",
    );
    expect(() => renderStaticHtml("<br>".repeat(5001))).toThrow("complexity");
  });

  it("maps document presentation onto a safe body wrapper and rejects empty output", () => {
    const fragment = renderStaticHtml(
      '<style>body{color:blue}:root{font-size:16px}</style><body style="padding:12px"><p>Readable</p></body>',
    );
    expect(fragment.querySelector(".lens-document-body")?.getAttribute("style")).toBe(
      "padding:12px",
    );
    expect(fragment.querySelector("style")?.textContent).toContain(
      ".content .lens-document-body{color:blue}",
    );
    expect(() => renderStaticHtml("<script>alert(1)</script>")).toThrow("no displayable text");
  });
});
