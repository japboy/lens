import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { build, type Plugin } from "vite";

export const HTML_MATH_MANIFEST = "html-math-assets.json";
export const HTML_MATH_RESOURCE_LIMIT = 450 * 1024;
export interface HtmlMathAsset {
  path: string;
  sha256: string;
  mime: "text/css" | "font/woff2";
  byteLength: number;
}
export interface HtmlMathAssets {
  resourceDigest: string;
  stylesheetPath: string;
  fontPaths: string[];
  files: HtmlMathAsset[];
}
const digest = (bytes: Uint8Array | string): string =>
  createHash("sha256").update(bytes).digest("hex");

/** Only pinned package CSS enters this build-time transformation, never author CSS. */
export async function createHtmlMathAssets(desktopRoot: string): Promise<{
  manifest: HtmlMathAssets;
  sources: Map<string, Buffer>;
}> {
  const katex = join(desktopRoot, "node_modules/katex/dist");
  const input = readFileSync(join(katex, "katex.css"), "utf8").replace(/\/\*[\s\S]*?\*\//gu, "");
  const fonts = new Map<string, Buffer>();
  const rules: string[] = [];
  let position = 0;
  for (const rule of input.matchAll(/([^{}]+)\{([^{}]*)\}/gu)) {
    if (input.slice(position, rule.index).trim()) throw new Error("Unsupported KaTeX CSS grammar");
    position = rule.index! + rule[0].length;
    const selector = rule[1]!.trim();
    let body = rule[2]!;
    if (selector === "@font-face") {
      const font = /src:\s*url\(fonts\/(KaTeX_[A-Za-z0-9-]+\.woff2)\)[^;]*;/u.exec(body);
      if (!font || fonts.has(font[1]!)) throw new Error("Unexpected KaTeX font source");
      const filename = font[1]!;
      fonts.set(filename, readFileSync(join(katex, "fonts", filename)));
      body = body.replace(font[0], `src: url("fonts/${filename}") format("woff2");`);
      // Rename only font-family declarations, never relative font filenames.
      body = body.replace(/(font-family:\s*")KaTeX_/gu, "$1LensHtml_KaTeX_");
      rules.push(`${selector}{${body}}`);
    } else {
      const scoped = selector
        .split(",")
        .map((part) => {
          const value = part.trim();
          if (value === "body") return ".lens-html-math";
          if (!value.startsWith(".katex")) throw new Error("Unsupported KaTeX selector");
          return `.lens-html-math ${value}`;
        })
        .join(",");
      rules.push(`${scoped}{${body.replaceAll("KaTeX_", "LensHtml_KaTeX_")}}`);
    }
  }
  if (input.slice(position).trim() || fonts.size !== 20)
    throw new Error("Incomplete pinned KaTeX stylesheet");
  const css =
    rules.join("\n") +
    `
.lens-html-math .katex-display { max-width: 100%; overflow-x: auto; overflow-y: hidden; padding-block: .25em; }
.lens-html-math .katex-display > .katex { text-align: start; }
`;
  const entry = join(desktopRoot, ".lens-html-math.css");
  const result = await build({
    configFile: false,
    root: desktopRoot,
    base: "./",
    logLevel: "error",
    plugins: [
      {
        name: "lens-scoped-math-css",
        resolveId(id) {
          if (id === entry) return entry;
        },
        load(id) {
          if (id === entry) return css.replaceAll('url("fonts/', `url("${katex}/fonts/`);
        },
      },
    ],
    build: {
      write: false,
      cssMinify: true,
      minify: true,
      assetsInlineLimit: 0,
      rolldownOptions: {
        input: entry,
        output: {
          assetFileNames: (asset) =>
            asset.names[0]?.endsWith(".css") ? "katex.css" : "fonts/[name][extname]",
        },
      },
    },
  });
  if (Array.isArray(result) || !("output" in result))
    throw new Error("Unexpected math CSS build output");
  const bundled = new Map<string, Buffer>();
  for (const file of result.output) {
    if (file.type !== "asset") throw new Error("Math resource build emitted executable code");
    bundled.set(file.fileName, Buffer.from(file.source));
  }
  if (bundled.size !== 21 || !bundled.has("katex.css"))
    throw new Error("Incomplete math CSS build");
  const relative = new Map<string, Buffer>([["katex.css", bundled.get("katex.css")!]]);
  for (const name of [...fonts.keys()].sort()) {
    const bytes = bundled.get(`fonts/${name}`);
    if (!bytes?.equals(fonts.get(name)!)) throw new Error("Bundling changed KaTeX font bytes");
    relative.set(`fonts/${name}`, bytes);
  }
  if (
    [...relative.values()].reduce((sum, bytes) => sum + bytes.length, 0) > HTML_MATH_RESOURCE_LIMIT
  ) {
    throw new Error("Math resource budget exceeded");
  }
  const hash = createHash("sha256");
  for (const [name, bytes] of relative) hash.update(name).update("\0").update(bytes).update("\0");
  const resourceDigest = hash.digest("hex");
  const root = `assets/html-math/${resourceDigest}`;
  const sources = new Map([...relative].map(([name, bytes]) => [`${root}/${name}`, bytes]));
  const files: HtmlMathAsset[] = [...sources].map(([path, bytes]) => ({
    path,
    sha256: digest(bytes),
    mime: path.endsWith(".css") ? "text/css" : "font/woff2",
    byteLength: bytes.length,
  }));
  const manifest: HtmlMathAssets = {
    resourceDigest,
    stylesheetPath: `${root}/katex.css`,
    fontPaths: files.filter((file) => file.mime === "font/woff2").map((file) => file.path),
    files,
  };
  return { manifest, sources };
}

/** Validate public math-only authority and each byte before admitting a generation. */
export function readHtmlMathManifest(directory: string): HtmlMathAssets {
  const manifest = JSON.parse(
    readFileSync(join(directory, HTML_MATH_MANIFEST), "utf8"),
  ) as HtmlMathAssets;
  if (
    !/^[a-f0-9]{64}$/u.test(manifest.resourceDigest) ||
    !Array.isArray(manifest.files) ||
    manifest.files.length !== 21 ||
    !Array.isArray(manifest.fontPaths) ||
    manifest.fontPaths.length !== 20
  ) {
    throw new Error("Invalid math asset manifest");
  }
  const root = `assets/html-math/${manifest.resourceDigest}`;
  if (manifest.stylesheetPath !== `${root}/katex.css`)
    throw new Error("Invalid math stylesheet path");
  const seen = new Set<string>();
  let size = 0;
  for (const file of manifest.files) {
    const font = new RegExp(`^${root}/fonts/KaTeX_[A-Za-z0-9-]+\\.woff2$`, "u").test(file.path);
    const css = file.path === manifest.stylesheetPath;
    if (
      (!font && !css) ||
      seen.has(file.path) ||
      file.mime !== (css ? "text/css" : "font/woff2") ||
      !/^[a-f0-9]{64}$/u.test(file.sha256) ||
      !Number.isSafeInteger(file.byteLength) ||
      file.byteLength <= 0
    )
      throw new Error("Invalid math asset entry");
    seen.add(file.path);
    const bytes = readFileSync(join(directory, file.path));
    if (bytes.length !== file.byteLength || digest(bytes) !== file.sha256)
      throw new Error("Math asset bytes do not match manifest");
    size += bytes.length;
  }
  if (
    size > HTML_MATH_RESOURCE_LIMIT ||
    new Set(manifest.fontPaths).size !== 20 ||
    manifest.fontPaths.some((path) => !seen.has(path) || path === manifest.stylesheetPath)
  )
    throw new Error("Invalid math font manifest");
  const hash = createHash("sha256");
  for (const path of [manifest.stylesheetPath, ...[...manifest.fontPaths].sort()]) {
    hash
      .update(path.slice(root.length + 1))
      .update("\0")
      .update(readFileSync(join(directory, path)))
      .update("\0");
  }
  if (hash.digest("hex") !== manifest.resourceDigest)
    throw new Error("Math resource digest mismatch");
  return manifest;
}

/** CORS authority is restricted to the admitted immutable public math byte set. */
export function htmlMathResponseHeaders(
  manifest: HtmlMathAssets,
  path: string,
  bytes: Uint8Array,
): Record<string, string> {
  const file = manifest.files.find((asset) => asset.path === path);
  if (!file) return {};
  if (file.byteLength !== bytes.byteLength || file.sha256 !== digest(bytes))
    throw new Error("Public math asset changed after publication");
  return { "Access-Control-Allow-Origin": "*", "X-Content-Type-Options": "nosniff" };
}

export async function htmlMathAssetsPlugin(desktopRoot: string): Promise<Plugin> {
  const { manifest, sources } = await createHtmlMathAssets(desktopRoot);
  const name = "virtual:lens-html-math-assets";
  let building = false;
  return {
    name: "lens-html-math-assets",
    configResolved(config) {
      building = config.command === "build";
    },
    resolveId(id) {
      if (id === name) return `\0${name}`;
    },
    load(id) {
      if (id === `\0${name}`) return `export const htmlMathAssets = ${JSON.stringify(manifest)};`;
    },
    buildStart() {
      if (!building) return;
      for (const [fileName, source] of sources) this.emitFile({ type: "asset", fileName, source });
      this.emitFile({
        type: "asset",
        fileName: HTML_MATH_MANIFEST,
        source: JSON.stringify(manifest) + "\n",
      });
    },
  };
}
