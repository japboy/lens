import { afterEach, describe, expect, it } from "vitest";
import { mkdtemp, mkdir, writeFile, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { execFileSync } from "node:child_process";
import { sourceInputs, sourceDigest, WORKSPACE_PACKAGE_PATHS } from "./source.ts";
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
  await write(join(original, "packages/adapter-mcp-apps-host/source.json"), '{"value":"original"}');
  return { original };
}
describe("sealed source input ownership", () => {
  it("seals the current tree after a tracked package source is deleted", async () => {
    const { original } = await fixture();
    execFileSync("git", ["init", "--quiet"], { cwd: original });
    execFileSync("git", ["add", "packages"], { cwd: original });
    const source = "packages/adapter-mcp-apps-host/source.json";
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
      await readFile(new URL("../../../../.gitignore", import.meta.url), "utf8"),
    );
    const before = sourceDigest(sourceInputs(original));
    for (const packagePath of WORKSPACE_PACKAGE_PATHS.filter(
      (path) => path !== "packages/typescript-config",
    )) {
      const cache = `${packagePath}/.build/cache/typescript/browser.tsbuildinfo`;
      await write(join(original, cache), "generated cache bytes");
      const inputs = sourceInputs(original);
      expect(inputs.has(cache)).toBe(false);
      expect(sourceDigest(inputs)).toBe(before);
    }
  });
  it("captures and hashes actual new package source edits", async () => {
    const { original } = await fixture();
    execFileSync("git", ["init", "--quiet"], { cwd: original });
    const before = sourceInputs(original);
    expect(before.has("packages/adapter-mcp-apps-host/source.json")).toBe(true);
    await write(
      join(original, "packages/adapter-math-renderer/math.ts"),
      "export const value = 73;",
    );
    const after = sourceInputs(original);
    expect(after.has("packages/adapter-math-renderer/math.ts")).toBe(true);
    expect(sourceDigest(after)).not.toBe(sourceDigest(before));
  });
});
