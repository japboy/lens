import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { parse } from "yaml";
import { TEST_DISCOVERY_EXCLUDES } from "../../scripts/typescript-test-paths.ts";
import { PAGE_ENTRIES } from "../../apps/desktop/src/page-entries.ts";
import rootTestConfig from "../../vitest.config.ts";

const root = fileURLToPath(new URL("../..", import.meta.url));
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
    expect((parse(workspace) as { packages: string[] }).packages).toEqual([
      "apps/desktop",
      "apps/ui-preview",
      "packages/ui",
      "packages/adapter-lit-prerenderer",
      "packages/typescript-config",
      "packages/adapter-mcp-apps-host",
      "packages/adapter-mcp-apps-view",
      "packages/adapter-math-renderer",
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
  it("discovers recursive collaboration cases without executing helpers or fixtures", () => {
    expect(rootTestConfig.root?.replace(/\/$/u, "")).toBe(root.replace(/\/$/u, ""));
    const projects = rootTestConfig.test?.projects;
    expect(Array.isArray(projects)).toBe(true);
    if (!Array.isArray(projects)) throw new Error("Expected explicit Vitest projects");
    const owners = projects.filter(
      (project): project is Exclude<typeof project, string | Function | Promise<unknown>> =>
        typeof project === "object" && project !== null && "test" in project,
    );
    expect(owners.map((project) => project.test?.name)).toEqual([
      "repository",
      "desktop",
      "ui",
      "adapter-lit-prerenderer",
      "ui-preview",
      "mcp-apps-host",
      "mcp-apps-view",
      "math-renderer",
    ]);
    expect(owners.map((project) => project.root?.replace(/\/$/u, ""))).toEqual([
      root.replace(/\/$/u, ""),
      ...[
        "apps/desktop",
        "packages/ui",
        "packages/adapter-lit-prerenderer",
        "apps/ui-preview",
        "packages/adapter-mcp-apps-host",
        "packages/adapter-mcp-apps-view",
        "packages/adapter-math-renderer",
      ].map((owner) => resolve(root, owner)),
    ]);
    expect(owners[0]?.test?.include).toContain("tests/**/*.ts");
    for (const project of owners) {
      expect(project.extends).toBe(
        project.test?.name === "desktop"
          ? resolve(root, "apps/desktop/vite.config.ts")
          : project.test?.name === "ui-preview"
            ? resolve(root, "apps/ui-preview/vite.config.ts")
            : false,
      );
      expect(project.test?.exclude).toEqual([...TEST_DISCOVERY_EXCLUDES]);
    }
    expect(read("tsconfig.node.json").include).toContain("tests/**/*.ts");
    for (const owner of [
      "apps/desktop",
      "packages/ui",
      "packages/adapter-lit-prerenderer",
      "apps/ui-preview",
      "packages/adapter-mcp-apps-host",
      "packages/adapter-mcp-apps-view",
      "packages/adapter-math-renderer",
    ]) {
      const tests = read(`${owner}/tsconfig.test.json`);
      expect(tests.include).toContain("tests/**/*.ts");
      expect(tests.include).toContain("src/**/*.test.ts");
      expect(tests.exclude).toEqual([...TEST_DISCOVERY_EXCLUDES]);
      const runtimeConfigs = ["apps/desktop", "packages/ui", "apps/ui-preview"].includes(owner)
        ? ["tsconfig.app.json"]
        : owner === "packages/adapter-lit-prerenderer"
          ? ["tsconfig.node.json"]
          : owner === "packages/adapter-math-renderer"
            ? ["tsconfig.browser.json", "tsconfig.node.json"]
            : ["tsconfig.browser.json"];
      for (const name of runtimeConfigs) {
        expect(
          config(`${owner}/${name}`).files.every(
            (path: string) => !path.endsWith(".test.ts") && !path.includes("/tests/"),
          ),
        ).toBe(true);
      }
    }
  });
  it("keeps the explicit fixture qualification surface outside packaged page entries", () => {
    expect(Object.values(PAGE_ENTRIES)).not.toContain("media-qualification.html");
    expect(
      readFileSync(resolve(root, "apps/desktop/src/media-qualification.html"), "utf8"),
    ).toContain('src="./media-qualification.ts"');
    expect(readFileSync(resolve(root, "apps/desktop/vite.config.ts"), "utf8")).toContain(
      "Object.entries(PAGE_ENTRIES)",
    );
  });
  it("selects the central owner explicitly from package scripts", () => {
    for (const [owner, project] of [
      ["apps/desktop", "desktop"],
      ["packages/ui", "ui"],
      ["packages/adapter-lit-prerenderer", "adapter-lit-prerenderer"],
      ["apps/ui-preview", "ui-preview"],
      ["packages/adapter-mcp-apps-host", "mcp-apps-host"],
      ["packages/adapter-mcp-apps-view", "mcp-apps-view"],
      ["packages/adapter-math-renderer", "math-renderer"],
    ]) {
      expect(read(`${owner}/package.json`).scripts.test).toBe(
        `vitest run --config ../../vitest.config.ts --project ${project}`,
      );
    }
  });
  it("keeps a single Vitest definition and leaves desktop production Vite test-free", () => {
    for (const owner of [
      "apps/desktop",
      "packages/ui",
      "packages/adapter-lit-prerenderer",
      "apps/ui-preview",
      "packages/adapter-mcp-apps-host",
      "packages/adapter-mcp-apps-view",
      "packages/adapter-math-renderer",
    ])
      expect(existsSync(resolve(root, owner, "vitest.config.ts"))).toBe(false);
    expect(readFileSync(resolve(root, "apps/desktop/vite.config.ts"), "utf8")).not.toMatch(
      /\btest\s*:/u,
    );
  });
});
