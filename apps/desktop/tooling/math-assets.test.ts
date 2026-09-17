import { readFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { BUILD_PATHS } from "./build-paths";

const output = resolve(BUILD_PATHS.tests);
const overlay = readFileSync(join(output, "overlay.html"), "utf8");

describe("bundled Markdown math assets", () => {
  it("ships the same local math fonts for document and shadow styles", () => {
    const stylesheets = [...overlay.matchAll(/<link\b[^>]*href="([^"]+\.css)"/g)].map((match) =>
      readFileSync(join(output, match[1]!.replace(/^\//, "")), "utf8"),
    );
    const fontUrls = (css: string) =>
      [...css.matchAll(/url\(["']?([^\s"')]*KaTeX_[^\s"')]+)["']?\)/g)].map((match) => match[1]!);
    const documentFonts = [...new Set(fontUrls(stylesheets.join("\n")))].sort();
    const shadowFonts = [...new Set(fontUrls(overlay))].sort();
    expect(documentFonts.length).toBeGreaterThan(0);
    expect(shadowFonts).toEqual(documentFonts);
    for (const url of documentFonts) {
      expect(url).toMatch(/^\/assets\/KaTeX_/);
      expect(existsSync(join(output, url.slice(1)))).toBe(true);
    }
    expect(overlay).toContain(".katex-mathml");
  });

  it.each(["settings", "about", "target-selection"])(
    "does not introduce math assets in %s",
    (view) => {
      const html = readFileSync(join(output, `${view}.html`), "utf8");
      expect(html).not.toContain("KaTeX_");
      const stylesheets = [...html.matchAll(/<link\b[^>]*href="([^"]+\.css)"/g)].map((match) =>
        readFileSync(join(output, match[1]!.replace(/^\//, "")), "utf8"),
      );
      expect(stylesheets.join("\n")).not.toContain("KaTeX_");
    },
  );
});
