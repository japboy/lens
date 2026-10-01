import { afterEach, describe, expect, it } from "vitest";
import { mkdtemp, mkdir, writeFile, rm, readFile, realpath, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";
import { linkGenerationDependencies } from "./workspace-dependencies.ts";
import { WORKSPACE_PACKAGE_PATHS } from "./source.ts";

const owned: string[] = [];
afterEach(async () => {
  await Promise.all(owned.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});
async function write(path: string, data: string) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, data);
}
async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "lens-sealed-package-"));
  owned.push(root);
  const original = join(root, "original"),
    staging = join(root, "staging");
  const entries: Array<[string, object]> = [
    [".", { name: "repo" }],
    [
      "apps/desktop",
      {
        name: "desktop",
        dependencies: { "adapter-mcp-apps-host": "workspace:*", third: "1.0.0" },
      },
    ],
    ...WORKSPACE_PACKAGE_PATHS.map((path): [string, object] => [
      path,
      { name: path.split("/")[1] },
    ]),
  ];
  for (const [path, manifest] of entries) {
    await write(join(original, path, "package.json"), JSON.stringify(manifest));
    await write(join(staging, path, "package.json"), JSON.stringify(manifest));
  }
  await write(join(original, "packages/adapter-mcp-apps-host/source.json"), '{"value":"original"}');
  await write(join(staging, "packages/adapter-mcp-apps-host/source.json"), '{"value":"sealed"}');
  await write(
    join(original, "node_modules/.pnpm/third@1.0.0/node_modules/third/package.json"),
    '{"name":"third","main":"value.json"}',
  );
  await write(
    join(original, "node_modules/.pnpm/third@1.0.0/node_modules/third/value.json"),
    '{"value":"third-party"}',
  );
  await mkdir(join(original, "apps/desktop/node_modules"), { recursive: true });
  await symlink(
    join(original, "node_modules/.pnpm/third@1.0.0/node_modules/third"),
    join(original, "apps/desktop/node_modules/third"),
    "dir",
  );
  return { original, staging };
}
describe("sealed workspace dependency resolution", () => {
  it("resolves workspace bytes in staging despite later original source mutation", async () => {
    const { original, staging } = await fixture();
    await linkGenerationDependencies(original, staging);
    await write(
      join(original, "packages/adapter-mcp-apps-host/source.json"),
      '{"value":"mutated"}',
    );
    const require = createRequire(join(staging, "apps/desktop/package.json"));
    const resolved = require.resolve("adapter-mcp-apps-host/source.json");
    expect(await realpath(resolved)).toBe(
      await realpath(join(staging, "packages/adapter-mcp-apps-host/source.json")),
    );
    expect(JSON.parse(await readFile(resolved, "utf8"))).toEqual({ value: "sealed" });
    expect(require("third")).toEqual({ value: "third-party" });
  });
  it("rejects a non-workspace local version instead of escaping through installed links", async () => {
    const { original, staging } = await fixture();
    await write(
      join(staging, "apps/desktop/package.json"),
      JSON.stringify({
        name: "desktop",
        dependencies: {
          "adapter-mcp-apps-host": "file:../../packages/adapter-mcp-apps-host",
        },
      }),
    );
    await expect(linkGenerationDependencies(original, staging)).rejects.toThrow(
      "Unsealed local dependency",
    );
  });
  it("rejects an installed third-party name pointing into another checkout", async () => {
    const { original, staging } = await fixture();
    const other = join(original, "../other-checkout/packages/foreign");
    await write(join(other, "package.json"), '{"name":"third"}');
    const installed = join(original, "apps/desktop/node_modules/third");
    await rm(installed);
    await symlink(other, installed, "dir");
    await expect(linkGenerationDependencies(original, staging)).rejects.toThrow(
      "Installed dependency escapes sealed workspace",
    );
  });
});
