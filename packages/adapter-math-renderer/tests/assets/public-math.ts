import { afterEach, describe, expect, it } from "vitest";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  readHtmlMathManifest,
  HTML_MATH_MANIFEST,
  HTML_MATH_RESOURCE_LIMIT,
} from "../../src/node/html-math-manifest.ts";
import { createHtmlMathAssets, inlineHtmlMathCss } from "../../src/node/html-math-assets.ts";

const desktop = resolve(".");
const temporary: string[] = [];
afterEach(() => {
  for (const path of temporary.splice(0)) rmSync(path, { recursive: true, force: true });
});
async function fixture() {
  const result = await createHtmlMathAssets(desktop);
  const directory = mkdtempSync(join(tmpdir(), "lens-math-assets-"));
  temporary.push(directory);
  for (const [file, bytes] of result.sources) {
    const path = join(directory, file);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, bytes);
  }
  writeFileSync(join(directory, HTML_MATH_MANIFEST), JSON.stringify(result.manifest));
  return { ...result, directory };
}
describe("closed math asset publication", () => {
  it("emits a deterministic closed CSS/font graph with exact package font bytes", async () => {
    const { manifest, sources, directory } = await fixture();
    expect((await createHtmlMathAssets(desktop)).manifest).toEqual(manifest);
    expect(readHtmlMathManifest(directory)).toEqual(manifest);
    expect(manifest.files).toHaveLength(21);
    expect(manifest.fontPaths).toHaveLength(20);
    expect(manifest.files.reduce((sum, file) => sum + file.byteLength, 0)).toBeLessThanOrEqual(
      HTML_MATH_RESOURCE_LIMIT,
    );
    for (const path of manifest.fontPaths)
      expect(sources.get(path)).toEqual(
        readFileSync(
          join(
            dirname(fileURLToPath(import.meta.resolve("katex/dist/katex.css"))),
            "fonts",
            basename(path),
          ),
        ),
      );
    const css = sources.get(manifest.stylesheetPath)!.toString();
    const urls = [...css.matchAll(/url\(["']?([^"')]+)["']?\)/gu)].map((match) => match[1]!);
    expect(urls).toHaveLength(20);
    for (const url of urls)
      expect(sources.has(join(dirname(manifest.stylesheetPath), url))).toBe(true);
    expect(css).not.toMatch(/data:|https?:|@import/u);
    const inline = inlineHtmlMathCss(manifest, sources);
    const inlineFonts = [...inline.matchAll(/url\("data:font\/woff2;base64,([^"]+)"\)/gu)].map(
      (match) => Buffer.from(match[1]!, "base64"),
    );
    expect(inlineFonts).toEqual(
      urls.map((url) => sources.get(join(dirname(manifest.stylesheetPath), url))),
    );
    expect(inline).not.toMatch(/https?:|@import|url\((?!"data:font\/woff2;base64,)/u);
    const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/gu)];
    for (const rule of rules.filter((rule) => rule[1]!.trim() === "@font-face"))
      expect(rule[2]).toMatch(/font-family:\s*"?LensHtml_KaTeX_/u);
    for (const rule of rules.filter((rule) => rule[1]!.trim() !== "@font-face"))
      for (const selector of rule[1]!.split(","))
        expect(selector.trim()).toMatch(/^\.lens-html-math(?:\s|$)/u);
  });
});
