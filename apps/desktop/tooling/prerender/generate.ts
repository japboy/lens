import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import {
  readFile,
  writeFile,
  mkdir,
  mkdtemp,
  rm,
  rename,
  symlink,
  readdir,
} from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { BUILD_PATHS, assertGenerationOutput } from "../build-paths.ts";
import { fileURLToPath } from "node:url";
import { build } from "vite";
import { PAGE_ENTRIES } from "../../src/page-entries.ts";
import { sourceInputs, sourceDigest } from "./source.ts";
import { verifyGeneration } from "./verify.ts";

const execute = promisify(execFile);
const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const repository = resolve(desktop, "../..");
async function filesUnder(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  return (
    await Promise.all(
      entries.map(async (entry) =>
        entry.isDirectory()
          ? (await filesUnder(join(directory, entry.name))).map((file) => join(entry.name, file))
          : [entry.name],
      ),
    )
  )
    .flat()
    .sort();
}

export async function generate(output: string, development = false): Promise<string> {
  assertGenerationOutput(desktop, output);
  const source = sourceInputs(repository);
  const generation = sourceDigest(source);
  const workspace = join(desktop, BUILD_PATHS.staging);
  await mkdir(workspace, { recursive: true });
  const staging = await mkdtemp(join(workspace, "generation-"));
  const app = join(staging, "apps/desktop");
  const clientRoot = join(app, "src");
  try {
    for (const [file, bytes] of source) {
      const destination = join(staging, file);
      await mkdir(dirname(destination), { recursive: true });
      await writeFile(destination, bytes);
    }
    await symlink(join(desktop, "node_modules"), join(app, "node_modules"), "dir");
    await symlink(join(repository, "node_modules"), join(staging, "node_modules"), "dir");
    const ssrOutput = join(app, ".render");
    await build({
      configFile: false,
      root: app,
      logLevel: "warn",
      build: {
        ssr: "tooling/prerender/render-entry.ts",
        outDir: ssrOutput,
        emptyOutDir: true,
        emitAssets: true,
        assetsInlineLimit: 0,
        minify: false,
        rolldownOptions: { external: ["lit", "@lit-labs/ssr"] },
      },
    });
    const renderedFile = join(ssrOutput, "views.json");
    await execute(process.execPath, [join(ssrOutput, "render-entry.js"), renderedFile], {
      cwd: app,
    });
    const base = development ? `/_generations/${generation}/` : "/";
    const rendered = JSON.parse(await readFile(renderedFile, "utf8")) as Record<string, string>;
    for (const [view, file] of Object.entries(PAGE_ENTRIES)) {
      const htmlPath = join(clientRoot, file);
      const envelope = await readFile(htmlPath, "utf8");
      const marker = `<!-- lens-prerender:${view} -->`;
      if (envelope.split(marker).length !== 2 || !rendered[view])
        throw new Error(`Invalid prerender outlet: ${file}`);
      await writeFile(
        htmlPath,
        envelope.replace(marker, rendered[view].replaceAll("/assets/", `${base}assets/`)),
      );
    }
    const client = join(app, ".client");
    await build({
      configFile: false,
      root: clientRoot,
      base,
      logLevel: "warn",
      build: {
        outDir: client,
        emptyOutDir: true,
        manifest: true,
        assetsInlineLimit: 0,
        minify: !development,
        sourcemap: development,
        rolldownOptions: {
          input: Object.fromEntries(
            Object.entries(PAGE_ENTRIES).map(([view, file]) => [view, join(clientRoot, file)]),
          ),
        },
      },
    });
    // SSR can emit assets that the initial browser graph does not reference.
    for (const file of await filesUnder(join(ssrOutput, "assets"))) {
      const bytes = await readFile(join(ssrOutput, "assets", file));
      const destination = join(client, "assets", file);
      let existing: Buffer | undefined;
      try {
        existing = await readFile(destination);
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      }
      if (existing && !existing.equals(bytes)) throw new Error(`Conflicting asset: ${file}`);
      if (!existing) {
        await mkdir(dirname(destination), { recursive: true });
        await writeFile(destination, bytes);
      }
    }
    for (const [view, file] of Object.entries(PAGE_ENTRIES)) {
      const built = await readFile(join(client, file), "utf8");
      const comments = (text: string) =>
        [...text.matchAll(/<!--[\s\S]*?-->/g)].map((match) => match[0]);
      const initial = await readFile(join(clientRoot, file), "utf8");
      if (JSON.stringify(comments(initial)) !== JSON.stringify(comments(built)))
        throw new Error(`Hydration markers changed: ${view}`);
      if (!built.includes('shadowrootmode="open"')) throw new Error(`Missing built DSD: ${view}`);
    }
    if (generation !== sourceDigest(sourceInputs(repository)))
      throw new Error("Source changed during generation");
    const manifest = Object.fromEntries(
      await Promise.all(
        (await filesUnder(client)).map(async (file) => [
          file.replaceAll("\\", "/"),
          createHash("sha256")
            .update(await readFile(join(client, file)))
            .digest("hex"),
        ]),
      ),
    );
    await writeFile(
      join(client, "generation.json"),
      JSON.stringify({ generation, files: manifest }) + "\n",
    );
    verifyGeneration(client, generation);
    await mkdir(dirname(output), { recursive: true });
    await rm(output, { recursive: true, force: true });
    await rename(client, output);
    return generation;
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}
