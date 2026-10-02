// @vitest-environment jsdom
// @vitest-environment-options {"jsdom":{"runScripts":"dangerously"}}
import { beforeAll, describe, expect, it } from "vitest";
import { prepareRichHtmlDocument as prepareDocument } from "../../src/rich-html-document";
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

describe("built-in document and closed math resources", () => {
  it("retains doctype, CSS and executable author bytes while rendering bounded math with inline local fonts", () => {
    const script =
      '<script>window.fixture=73; document.getElementById("change").onclick=()=>document.getElementById("value").textContent="74";</script>';
    const style = "<style>body{background:rgb(10,20,30)} #value{color:blue}</style>";
    const source = `<!doctype html><html><head><meta charset="utf-8">${style}</head><body><p>\\(x^2\\)</p><p>\\[\\frac{a}{b}\\]</p><button id="change">Change</button><output id="value">73</output>${script}</body></html>`;
    const prepared = prepareRichHtmlDocument(source);
    expect(prepared.startsWith("<!doctype html>")).toBe(true);
    expect(prepared).toContain(script);
    expect(prepared).toContain(style);
    const errors: string[] = [];
    const dom = documentFixture(prepared, errors);
    try {
      expect(errors).toEqual([]);
      expect(dom.window.fixture).toBe(73);
      dom.window.document.getElementById("change")!.click();
      expect(dom.window.document.getElementById("value")!.textContent).toBe("74");
      expect(dom.window.document.querySelectorAll(".lens-html-math math")).toHaveLength(2);
      expect(dom.window.document.querySelectorAll(".katex-display")).toHaveLength(1);
      const css = dom.window.document.querySelector("style[data-lens-math]")!.textContent!;
      expect([...css.matchAll(/url\("data:font\/woff2;base64,[^"]+"\)/gu)]).toHaveLength(20);
      expect(css).not.toMatch(/https?:|@import|url\((?!"data:font\/woff2;base64,)/u);
      expect(dom.window.document.querySelectorAll("link, script[src]")).toHaveLength(0);
    } finally {
      dom.close();
    }
  });
});
