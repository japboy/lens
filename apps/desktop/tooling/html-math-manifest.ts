import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";

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
