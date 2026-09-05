import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = fileURLToPath(new URL("..", import.meta.url));
const read = (path: string) => JSON.parse(readFileSync(resolve(root, path), "utf8"));
const config = (path: string) =>
  JSON.parse(
    execFileSync("pnpm", ["exec", "tsc", "--showConfig", "-p", path], {
      cwd: root,
      encoding: "utf8",
    }),
  );

describe("shared TypeScript configuration", () => {
  it("lists only explicit non-root members in the pnpm workspace declaration", () => {
    const workspace = readFileSync(resolve(root, "pnpm-workspace.yaml"), "utf8");
    expect(workspace.split("\n").filter((line) => line.startsWith("  - "))).toEqual([
      '  - "apps/desktop"',
      '  - "packages/typescript-config"',
    ]);
  });

  it("keeps shared options separate from consumer file selection and references", () => {
    for (const path of ["base.json", "node.json"]) {
      const shared = read(`packages/typescript-config/${path}`);
      expect(shared.$schema).toBe("https://json.schemastore.org/tsconfig");
      expect(shared.files).toBeUndefined();
      expect(shared.include).toBeUndefined();
      expect(shared.exclude).toBeUndefined();
      expect(shared.references).toBeUndefined();
      expect(shared.compilerOptions.types).toBeUndefined();
    }
    for (const path of ["tsconfig.json", "apps/desktop/tsconfig.json"]) {
      expect(Object.keys(read(path)).sort()).toEqual(["files", "references"]);
    }
  });

  it.each([
    "tsconfig.node.json",
    "apps/desktop/tsconfig.node.json",
    "apps/desktop/tsconfig.app.json",
    "apps/desktop/tsconfig.test.json",
  ])("resolves strict/no-emit settings via the workspace package: %s", (path) => {
    expect(config(path).compilerOptions).toMatchObject({
      strict: true,
      noEmit: true,
      skipLibCheck: true,
    });
  });

  it("keeps Node globals out of browser sources and admits executable task files", () => {
    const browser = config("apps/desktop/tsconfig.app.json");
    expect(browser.compilerOptions.types).toEqual(["vite/client"]);
    expect(browser.files.every((path: string) => !path.endsWith(".test.ts"))).toBe(true);
    expect(config("apps/desktop/tsconfig.test.json").compilerOptions.types).toEqual([
      "vite/client",
      "node",
    ]);
    const tooling = config("tsconfig.node.json");
    expect(tooling.compilerOptions).toMatchObject({ types: ["node"], erasableSyntaxOnly: true });
    expect(tooling.files).toContain("./mise-tasks/check/identity.ts");
  });
});
