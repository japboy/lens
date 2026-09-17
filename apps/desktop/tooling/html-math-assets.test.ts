import { afterEach, describe, expect, it } from "vitest";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import {
  createHtmlMathAssets,
  readHtmlMathManifest,
  htmlMathResponseHeaders,
  HTML_MATH_MANIFEST,
  HTML_MATH_RESOURCE_LIMIT,
} from "./html-math-assets.ts";

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
describe("public HTML math asset generation", () => {
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
        readFileSync(join(desktop, "node_modules/katex/dist/fonts", basename(path))),
      );
    const css = sources.get(manifest.stylesheetPath)!.toString();
    const urls = [...css.matchAll(/url\(["']?([^"')]+)["']?\)/gu)].map((match) => match[1]!);
    expect(urls).toHaveLength(20);
    for (const url of urls)
      expect(sources.has(join(dirname(manifest.stylesheetPath), url))).toBe(true);
    expect(css).not.toMatch(/data:|https?:|@import/u);
    const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/gu)];
    for (const rule of rules.filter((rule) => rule[1]!.trim() === "@font-face"))
      expect(rule[2]).toMatch(/font-family:\s*"?LensHtml_KaTeX_/u);
    for (const rule of rules.filter((rule) => rule[1]!.trim() !== "@font-face"))
      for (const selector of rule[1]!.split(","))
        expect(selector.trim()).toMatch(/^\.lens-html-math(?:\s|$)/u);
  });
  it("rejects changed bytes before publication or serving", async () => {
    const { manifest, sources, directory } = await fixture();
    const path = manifest.fontPaths[0]!;
    const changed = Buffer.from(sources.get(path)!);
    changed[0] ^= 1;
    expect(() => htmlMathResponseHeaders(manifest, path, changed)).toThrow(
      "changed after publication",
    );
    writeFileSync(join(directory, path), changed);
    expect(() => readHtmlMathManifest(directory)).toThrow("bytes do not match");
  });
  it("grants CORS only to an exact public entry, never HTML, code, manifest, or sibling resources", async () => {
    const { manifest, sources } = await createHtmlMathAssets(desktop);
    expect(
      htmlMathResponseHeaders(
        manifest,
        manifest.stylesheetPath,
        sources.get(manifest.stylesheetPath)!,
      ),
    ).toEqual({ "Access-Control-Allow-Origin": "*", "X-Content-Type-Options": "nosniff" });
    for (const path of [
      "overlay.html",
      "assets/overlay.js",
      HTML_MATH_MANIFEST,
      "assets/html-math/other.css",
      `/${manifest.stylesheetPath}`,
      `${manifest.stylesheetPath}?x=1`,
    ])
      expect(htmlMathResponseHeaders(manifest, path, Buffer.from("private"))).toEqual({});
  });
  it("rejects manifest attempts to expand the asset namespace", async () => {
    const { manifest, directory } = await fixture();
    manifest.files[0]!.path = "../private.css";
    writeFileSync(join(directory, HTML_MATH_MANIFEST), JSON.stringify(manifest));
    expect(() => readHtmlMathManifest(directory)).toThrow("Invalid math asset entry");
  });
});
