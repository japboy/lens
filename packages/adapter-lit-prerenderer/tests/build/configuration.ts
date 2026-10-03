import { afterEach, expect, it } from "vitest";
import { mkdtemp, mkdir, writeFile, readFile, rm, symlink, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { generateGeneration } from "../../src/generate.ts";
import {
  sourceInputs,
  workspaceDirectories,
  type SourceSnapshotContract,
} from "../../src/source-snapshot.ts";
import { verifyGeneration } from "../../src/verify.ts";
import { linkGenerationDependencies } from "../../src/workspace-dependencies.ts";
import type { PrerenderContract } from "../../src/contract.ts";
const owned: string[] = [];
afterEach(async () => {
  await Promise.all(owned.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});
async function write(path: string, bytes: string) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, bytes);
}
it("builds independent Lit roots and a renamed renderer without application assets or native imports", async () => {
  const repository = await mkdtemp(join(tmpdir(), "lit-independent-consumer-"));
  owned.push(repository);
  const packageRoot = fileURLToPath(new URL("../../", import.meta.url));
  await write(join(repository, ".gitignore"), "node_modules/\n.build/\n");
  await write(
    join(repository, "package.json"),
    JSON.stringify({ name: "fixture", type: "module" }),
  );
  await write(
    join(repository, "apps/catalog/package.json"),
    JSON.stringify({
      name: "catalog",
      type: "module",
      dependencies: { lit: "3.3.3", "@lit-labs/ssr": "4.1.0" },
    }),
  );
  await mkdir(join(repository, "node_modules"));
  await symlink(
    await realpath(join(packageRoot, "../../node_modules/.pnpm")),
    join(repository, "node_modules/.pnpm"),
    "dir",
  );
  for (const dependency of ["lit", "@lit-labs/ssr"]) {
    const destination = join(repository, "apps/catalog/node_modules", dependency);
    await mkdir(dirname(destination), { recursive: true });
    await symlink(
      await realpath(join(packageRoot, "node_modules", dependency)),
      destination,
      "dir",
    );
  }
  await write(
    join(repository, "apps/catalog/src/root.ts"),
    `import { LitElement, html } from "lit";
class CatalogCard extends LitElement { render() { return html\`<p>Independent catalog</p>\`; } }
customElements.define("catalog-card", CatalogCard);`,
  );
  await write(join(repository, "apps/catalog/src/client.ts"), 'import "./root.ts";');
  await write(
    join(repository, "apps/catalog/catalog-renderer.ts"),
    `import { writeFileSync } from "node:fs";
import { render } from "@lit-labs/ssr";
import { html } from "lit";
import "./src/root.ts";
const template = html\`<catalog-card defer-hydration></catalog-card>\`;
const first = Array.from(render(template)).join("");
if (first !== Array.from(render(template)).join("")) throw Error("Unstable root");
writeFileSync(process.argv[2]!, JSON.stringify({ showcase: first }));`,
  );
  const envelope =
    '<!doctype html><html><body><!-- catalog-outlet --><script type="module" src="./client.ts"></script></body></html>';
  await write(join(repository, "apps/catalog/src/showcase.html"), envelope);
  execFileSync("git", ["init", "--quiet"], { cwd: repository });
  const snapshot: SourceSnapshotContract = {
    repository,
    applicationPath: "apps/catalog",
    members: [{ directory: "." }, { directory: "apps/catalog" }],
    inputs: [".gitignore", "apps/catalog"],
  };
  const verification = {
    pages: {
      showcase: {
        html: "showcase.html",
        rootTag: "catalog-card",
        outlet: "<!-- catalog-outlet -->",
      },
    },
    resources: () => [],
  };
  const output = join(repository, ".build/output");
  const contract: PrerenderContract = {
    applicationPath: "apps/catalog",
    clientPath: "src",
    stagingRoot: join(repository, ".build/staging"),
    renderEntry: "catalog-renderer.ts",
    rendererBundleFilename: "different-renderer.js",
    pages: verification.pages,
    sourceInputs: () => sourceInputs(snapshot),
    linkDependencies: (staging) =>
      linkGenerationDependencies(repository, staging, workspaceDirectories(snapshot)),
    plugins: async () => [],
    verify: (directory, generation) => verifyGeneration(directory, verification, generation),
    assertOutput: (path) => {
      if (resolve(path) !== output) throw Error("Unowned output");
    },
  };
  const generation = await generateGeneration(contract, output, true);
  const document = await readFile(join(output, "showcase.html"), "utf8");
  expect(document).toContain("Independent catalog");
  expect(document).toContain('shadowrootmode="open"');
  expect(document).toContain(`/_generations/${generation}/`);
  verifyGeneration(output, verification, generation);
  await writeFile(join(output, "showcase.html"), document + "tampered");
  expect(() => verifyGeneration(output, verification, generation)).toThrow("file set or digest");
  await expect(generateGeneration(contract, join(repository, "unowned"))).rejects.toThrow(
    "Unowned output",
  );
  await write(
    join(repository, "apps/catalog/src/showcase.html"),
    envelope.replace("<!-- catalog-outlet -->", "<!-- catalog-outlet --><!-- catalog-outlet -->"),
  );
  await expect(generateGeneration(contract, output)).rejects.toThrow("Invalid prerender outlet");
  await write(join(repository, "apps/catalog/src/showcase.html"), envelope);
  let mutated = false;
  contract.plugins = async () => [
    {
      name: "mutate-original-after-sealing",
      async buildStart() {
        if (!mutated) {
          mutated = true;
          const root = join(repository, "apps/catalog/src/root.ts");
          await writeFile(
            root,
            (await readFile(root, "utf8")) + "\n// changed during generation\n",
          );
        }
      },
    },
  ];
  await expect(generateGeneration(contract, output)).rejects.toThrow(
    "Source changed during generation",
  );
}, 30_000);
