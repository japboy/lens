import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFile, writeFile, mkdir, mkdtemp, rm, rename, readdir } from "node:fs/promises";
import { dirname, join } from "node:path";
import { build } from "vite";
import type { PrerenderContract } from "./contract.ts";
import { sourceDigest } from "./source-snapshot.ts";
const execute = promisify(execFile);
async function filesUnder(directory: string, optional = false): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true }).catch(
    (error: NodeJS.ErrnoException) => {
      if (optional && error.code === "ENOENT") return [];
      throw error;
    },
  );
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

export async function generateGeneration(
  contract: PrerenderContract,
  output: string,
  development = false,
): Promise<string> {
  contract.assertOutput(output);
  const source = contract.sourceInputs();
  const generation = sourceDigest(source);
  const workspace = contract.stagingRoot;
  await mkdir(workspace, { recursive: true });
  const staging = await mkdtemp(join(workspace, "generation-"));
  const app = join(staging, contract.applicationPath);
  const clientRoot = join(app, contract.clientPath);
  try {
    for (const [file, bytes] of source) {
      const destination = join(staging, file);
      await mkdir(dirname(destination), { recursive: true });
      await writeFile(destination, bytes);
    }
    await contract.linkDependencies(staging);
    const ssrOutput = join(app, ".render");
    await build({
      configFile: false,
      root: app,
      plugins: await contract.plugins(app),
      logLevel: "warn",
      build: {
        ssr: contract.renderEntry,
        outDir: ssrOutput,
        emptyOutDir: true,
        emitAssets: true,
        assetsInlineLimit: 0,
        minify: false,
        rolldownOptions: {
          external: ["lit", "@lit-labs/ssr"],
          output: { entryFileNames: contract.rendererBundleFilename },
        },
      },
    });
    const renderedFile = join(ssrOutput, "views.json");
    await execute(
      process.execPath,
      [join(ssrOutput, contract.rendererBundleFilename), renderedFile],
      {
        cwd: app,
      },
    );
    const base = development ? `/_generations/${generation}/` : "/";
    const rendered = JSON.parse(await readFile(renderedFile, "utf8")) as Record<string, string>;
    for (const [view, page] of Object.entries(contract.pages)) {
      const file = page.html;
      const htmlPath = join(clientRoot, file);
      const envelope = await readFile(htmlPath, "utf8");
      const marker = page.outlet;
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
      plugins: await contract.plugins(app),
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
            Object.entries(contract.pages).map(([view, page]) => [
              view,
              join(clientRoot, page.html),
            ]),
          ),
        },
      },
    });
    // SSR can emit assets that the initial browser graph does not reference.
    for (const file of await filesUnder(join(ssrOutput, "assets"), true)) {
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
    for (const [view, page] of Object.entries(contract.pages)) {
      const file = page.html;
      const built = await readFile(join(client, file), "utf8");
      const comments = (text: string) =>
        [...text.matchAll(/<!--[\s\S]*?-->/g)].map((match) => match[0]);
      const initial = await readFile(join(clientRoot, file), "utf8");
      if (JSON.stringify(comments(initial)) !== JSON.stringify(comments(built)))
        throw new Error(`Hydration markers changed: ${view}`);
      if (!built.includes('shadowrootmode="open"')) throw new Error(`Missing built DSD: ${view}`);
    }
    if (generation !== sourceDigest(contract.sourceInputs()))
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
    contract.verify(client, generation);
    await mkdir(dirname(output), { recursive: true });
    await rm(output, { recursive: true, force: true });
    await rename(client, output);
    return generation;
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}
