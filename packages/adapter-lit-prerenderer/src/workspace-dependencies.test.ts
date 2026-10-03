import { afterEach, describe, expect, it } from "vitest";
import { mkdtemp, mkdir, writeFile, rm, readFile, realpath, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";
import { linkGenerationDependencies } from "./workspace-dependencies.ts";
const WORKSPACE_PACKAGE_PATHS = ["packages/ui", "packages/math", "packages/config"];
const DIRECTORIES = [".", "apps/consumer", ...WORKSPACE_PACKAGE_PATHS];

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
      "apps/consumer",
      {
        name: "desktop",
        dependencies: { ui: "workspace:*", third: "1.0.0" },
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
  await write(join(original, "packages/ui/source.json"), '{"value":"original"}');
  await write(join(staging, "packages/ui/source.json"), '{"value":"sealed"}');
  await write(
    join(original, "node_modules/.pnpm/third@1.0.0/node_modules/third/package.json"),
    '{"name":"third","main":"value.json"}',
  );
  await write(
    join(original, "node_modules/.pnpm/third@1.0.0/node_modules/third/value.json"),
    '{"value":"third-party"}',
  );
  await mkdir(join(original, "apps/consumer/node_modules"), { recursive: true });
  await symlink(
    join(original, "node_modules/.pnpm/third@1.0.0/node_modules/third"),
    join(original, "apps/consumer/node_modules/third"),
    "dir",
  );
  return { original, staging };
}
describe("sealed workspace dependency resolution", () => {
  it("resolves workspace bytes in staging despite later original source mutation", async () => {
    const { original, staging } = await fixture();
    await linkGenerationDependencies(original, staging, DIRECTORIES);
    await write(join(original, "packages/ui/source.json"), '{"value":"mutated"}');
    const require = createRequire(join(staging, "apps/consumer/package.json"));
    const resolved = require.resolve("ui/source.json");
    expect(await realpath(resolved)).toBe(await realpath(join(staging, "packages/ui/source.json")));
    expect(JSON.parse(await readFile(resolved, "utf8"))).toEqual({ value: "sealed" });
    expect(require("third")).toEqual({ value: "third-party" });
  });
  it("rejects a non-workspace local version instead of escaping through installed links", async () => {
    const { original, staging } = await fixture();
    await write(
      join(staging, "apps/consumer/package.json"),
      JSON.stringify({
        name: "desktop",
        dependencies: {
          ui: "file:../../packages/ui",
        },
      }),
    );
    await expect(linkGenerationDependencies(original, staging, DIRECTORIES)).rejects.toThrow(
      "Unsealed local dependency",
    );
  });
  it("rejects an installed third-party name pointing into another checkout", async () => {
    const { original, staging } = await fixture();
    const other = join(original, "../other-checkout/packages/foreign");
    await write(join(other, "package.json"), '{"name":"third"}');
    const installed = join(original, "apps/consumer/node_modules/third");
    await rm(installed);
    await symlink(other, installed, "dir");
    await expect(linkGenerationDependencies(original, staging, DIRECTORIES)).rejects.toThrow(
      "Installed dependency escapes sealed workspace",
    );
  });
});
