import { afterEach, describe, expect, it } from "vitest";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { execFileSync } from "node:child_process";
import { sourceInputs as capture, sourceDigest, workspaceDirectories } from "./source-snapshot.ts";
const WORKSPACE_PACKAGE_PATHS = ["packages/ui", "packages/math", "packages/config"];
const contract = (repository: string) => ({
  repository,
  applicationPath: "apps/consumer",
  members: [".", "apps/consumer", ...WORKSPACE_PACKAGE_PATHS].map((directory) => ({ directory })),
  inputs: [".gitignore", "apps/consumer/src"],
});
const sourceInputs = (repository: string) => capture(contract(repository));
const owned: string[] = [];
afterEach(async () => {
  await Promise.all(owned.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});
async function write(path: string, data: string) {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, data);
}
async function fixture() {
  const original = await mkdtemp(join(tmpdir(), "lens-source-inputs-"));
  owned.push(original);
  for (const path of WORKSPACE_PACKAGE_PATHS)
    await write(join(original, path, "package.json"), JSON.stringify({ name: path.split("/")[1] }));
  await write(
    join(original, "package.json"),
    JSON.stringify({ name: "repo", devDependencies: { config: "workspace:*" } }),
  );
  await write(
    join(original, "apps/consumer/package.json"),
    JSON.stringify({ name: "consumer", dependencies: { ui: "workspace:*", math: "workspace:*" } }),
  );
  await write(join(original, "packages/ui/source.json"), '{"value":"original"}');
  return { original };
}
describe("sealed source input ownership", () => {
  it("seals the current tree after a tracked package source is deleted", async () => {
    const { original } = await fixture();
    execFileSync("git", ["init", "--quiet"], { cwd: original });
    execFileSync("git", ["add", "packages"], { cwd: original });
    const source = "packages/ui/source.json";
    const before = sourceDigest(sourceInputs(original));
    await rm(join(original, source));
    const inputs = sourceInputs(original);
    expect(inputs.has(source)).toBe(false);
    expect(sourceDigest(inputs)).not.toBe(before);
  });
  it("excludes generated shared-package caches using repository ignore policy", async () => {
    const { original } = await fixture();
    execFileSync("git", ["init", "--quiet"], { cwd: original });
    await write(
      join(original, ".gitignore"),
      WORKSPACE_PACKAGE_PATHS.map((path) => `/${path}/.build/`).join("\n") + "\n",
    );
    const before = sourceDigest(sourceInputs(original));
    for (const packagePath of WORKSPACE_PACKAGE_PATHS.filter(
      (path) => path !== "packages/config",
    )) {
      const cache = `${packagePath}/.build/cache/typescript/browser.tsbuildinfo`;
      await write(join(original, cache), "generated cache bytes");
      const inputs = sourceInputs(original);
      expect(inputs.has(cache)).toBe(false);
      expect(sourceDigest(inputs)).toBe(before);
    }
  });
  it("follows declared transitive members and excludes unrelated workspace members", async () => {
    const { original } = await fixture();
    await write(join(original, "packages/unused/package.json"), JSON.stringify({ name: "unused" }));
    const settings = contract(original);
    settings.members.push({ directory: "packages/unused" });
    expect(workspaceDirectories(settings)).toEqual([
      ".",
      "apps/consumer",
      "packages/config",
      "packages/math",
      "packages/ui",
    ]);
    await write(
      join(original, "packages/ui/package.json"),
      JSON.stringify({ name: "ui", dependencies: { unused: "workspace:*" } }),
    );
    expect(workspaceDirectories(settings)).toContain("packages/unused");
  });
  it("rejects duplicate workspace identities before resolving a dependency closure", async () => {
    const { original } = await fixture();
    const settings = contract(original);
    settings.members.push({ directory: "packages/ui" });
    expect(() => workspaceDirectories(settings)).toThrow("Duplicate workspace member identity");
  });
  it("captures and hashes actual new package source edits", async () => {
    const { original } = await fixture();
    execFileSync("git", ["init", "--quiet"], { cwd: original });
    const before = sourceInputs(original);
    expect(before.has("packages/ui/source.json")).toBe(true);
    await write(join(original, "packages/math/math.ts"), "export const value = 73;");
    const after = sourceInputs(original);
    expect(after.has("packages/math/math.ts")).toBe(true);
    expect(sourceDigest(after)).not.toBe(sourceDigest(before));
  });
});
