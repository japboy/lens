import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { join, posix } from "node:path";
import { PAGE_ENTRIES } from "../../src/page-entries.ts";

export function generationFiles(directory: string): Record<string, string> {
  const result: Record<string, string> = Object.create(null);
  function visit(prefix: string): void {
    for (const entry of readdirSync(join(directory, prefix), { withFileTypes: true })) {
      const file = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) visit(file);
      else if (entry.isFile()) {
        if (file !== "generation.json")
          result[file] = createHash("sha256")
            .update(readFileSync(join(directory, file)))
            .digest("hex");
      } else throw new Error("Generated assets must be regular files");
    }
  }
  visit("");
  return result;
}

/** Validate the complete generation before publication or source-bound artifact transfer. */
export function verifyGeneration(directory: string, expectedGeneration?: string): void {
  const metadata = JSON.parse(readFileSync(join(directory, "generation.json"), "utf8")) as {
    generation: string;
    files: Record<string, string>;
  };
  if (
    !/^[a-f0-9]{64}$/.test(metadata.generation) ||
    (expectedGeneration !== undefined && metadata.generation !== expectedGeneration)
  )
    throw new Error("Generated artifact source does not match the current inputs");
  const files = generationFiles(directory);
  const names = Object.keys(files).sort();
  if (
    JSON.stringify(names) !== JSON.stringify(Object.keys(metadata.files).sort()) ||
    names.some((file) => files[file] !== metadata.files[file])
  )
    throw new Error("Generated artifact file set or digest does not match");

  function requireAsset(url: string, from: string): void {
    if (/^(?:data:|https?:|#)/.test(url)) return;
    const path = decodeURIComponent(url.split(/[?#]/, 1)[0]!);
    const generationPrefix = `/_generations/${metadata.generation}/`;
    if (path.startsWith("/_generations/") && !path.startsWith(generationPrefix))
      throw new Error(`Mixed generation URL: ${url}`);
    const file = path.startsWith(generationPrefix)
      ? path.slice(generationPrefix.length)
      : path.startsWith("/")
        ? path.slice(1)
        : posix.normalize(posix.join(posix.dirname(from), path));
    if (!Object.hasOwn(files, file)) throw new Error(`Missing generated asset: ${url} in ${from}`);
  }

  for (const [view, file] of Object.entries(PAGE_ENTRIES)) {
    if (!Object.hasOwn(files, file)) throw new Error(`Missing generated entry: ${file}`);
    const html = readFileSync(join(directory, file), "utf8");
    if (
      (html.match(/shadowrootmode="open"/g) ?? []).length !== 1 ||
      !html.includes(`<lens-${view}-view`) ||
      !html.includes("defer-hydration") ||
      !html.includes("<!--lit-part ") ||
      !html.includes("<!--/lit-part-->") ||
      html.includes("lens-prerender:")
    )
      throw new Error(`Invalid generated DSD: ${view}`);
    for (const match of html.matchAll(/\b(?:src|href)="([^"]+)"/g)) requireAsset(match[1]!, file);
  }
  // Root styles are inline in DSD; document styles and module imports form the remaining graph.
  for (const file of names.filter((name) => /\.(?:html|css)$/.test(name))) {
    const text = readFileSync(join(directory, file), "utf8");
    for (const match of text.matchAll(/url\(\s*["']?([^\s"')]+)["']?\s*\)/g))
      requireAsset(match[1]!, file);
  }
  const manifest = JSON.parse(
    readFileSync(join(directory, ".vite/manifest.json"), "utf8"),
  ) as Record<
    string,
    {
      file: string;
      imports?: string[];
      dynamicImports?: string[];
      css?: string[];
      assets?: string[];
    }
  >;
  for (const [key, chunk] of Object.entries(manifest)) {
    for (const file of [chunk.file, ...(chunk.css ?? []), ...(chunk.assets ?? [])])
      requireAsset("/" + file, ".vite/manifest.json");
    for (const dependency of [...(chunk.imports ?? []), ...(chunk.dynamicImports ?? [])])
      if (!Object.hasOwn(manifest, dependency))
        throw new Error(`Missing manifest dependency: ${key} -> ${dependency}`);
  }
}
