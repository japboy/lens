import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { sourcePaths } from "adapter-lit-prerenderer/source-snapshot";
import { sourceContract, sourceInputs, sourceWatchRoots } from "./source.ts";

const repository = fileURLToPath(new URL("../../../../", import.meta.url));

describe("Desktop source and development watch contract", () => {
  it("watches every sealed source file and every declared scope, including artifact bootstrap", () => {
    const roots = sourceWatchRoots(repository);
    expect(roots).toHaveLength(1);
    expect(roots[0]!.directory).toBe(repository.replace(/\/$/u, ""));
    const { accepts } = roots[0]!;
    for (const path of sourceInputs(repository).keys()) expect(accepts(path)).toBe(true);
    for (const path of sourcePaths(sourceContract(repository))) {
      expect(accepts(path)).toBe(true);
      expect(accepts(`${path}/new-source.ts`)).toBe(true);
    }
    expect(accepts("scripts/no-install-workspace.ts")).toBe(true);
    expect(accepts("apps/desktop/agent-icons/claude.png")).toBe(true);
  });

  it("rejects caches, installed dependencies, unrelated owners and sibling prefixes", () => {
    const { accepts } = sourceWatchRoots(repository)[0]!;
    for (const path of [
      "apps/desktop/.build/webview/about.html",
      "packages/ui/.build/cache/typescript/app.tsbuildinfo",
      "packages/ui/node_modules/lit/index.js",
      "node_modules/.pnpm/lock.yaml",
      "apps/desktop/src-unrelated/example.ts",
      "apps/ui-preview/src/main.ts",
      "scripts/no-install-workspace.ts.backup",
    ])
      expect(accepts(path)).toBe(false);
  });
});
