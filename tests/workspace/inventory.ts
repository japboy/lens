import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  inspectWorkspace,
  repositoryPath,
  validateInventory,
} from "../../mise-tasks/check/boundaries.ts";
import type {
  CargoDependency,
  CargoInventory,
  PnpmManifest,
  PnpmMember,
} from "../../mise-tasks/check/boundaries.ts";
import { MEMBERS } from "../../scripts/workspace-policy.ts";

const root = fileURLToPath(new URL("../..", import.meta.url));
const baseline = inspectWorkspace(root);
const pnpm: PnpmMember[] = JSON.parse(
  execFileSync("pnpm", ["list", "--recursive", "--depth", "-1", "--json"], {
    cwd: root,
    encoding: "utf8",
  }),
);
const manifests = new Map(
  pnpm.map((member) => [
    repositoryPath(root, member.path),
    JSON.parse(readFileSync(resolve(member.path, "package.json"), "utf8")) as PnpmManifest,
  ]),
);

function fixture() {
  const cargo = structuredClone(baseline.cargo);
  const js = structuredClone(pnpm);
  const jsManifests = structuredClone(manifests);
  const paths = [...baseline.paths];
  return {
    cargo,
    js,
    jsManifests,
    paths,
    check: () => validateInventory(root, cargo, js, jsManifests, paths),
  };
}

function member(cargo: CargoInventory, name: string) {
  const result = cargo.packages.find((entry) => entry.name === name);
  if (!result) throw new Error(`Missing fixture member ${name}`);
  return result;
}

describe("workspace identities and all-kind dependency boundaries", () => {
  it("checks actual Cargo and pnpm discovery without conflating desktop identities", () => {
    expect(() => fixture().check()).not.toThrow();
    expect(baseline.cargo.packages).toHaveLength(6);
    expect(pnpm).toHaveLength(6);
    expect(
      MEMBERS.filter((entry) => entry.name === "desktop").map((entry) => entry.ecosystem),
    ).toEqual(["pnpm", "cargo"]);
  });

  it.each([null, "dev", "build"] as const)(
    "rejects reverse local dependency in %s section",
    (kind) => {
      const f = fixture();
      member(f.cargo, "domain").dependencies.push({
        name: "usecase",
        kind,
        target: null,
        rename: null,
        source: null,
        optional: false,
        path: resolve(root, "packages/usecase"),
      });
      expect(f.check).toThrow("forbidden dependency");
    },
  );

  it.each(['cfg(target_os = "macos")', 'cfg(target_os = "windows")'])(
    "checks inactive-target edges: %s",
    (target) => {
      const f = fixture();
      member(f.cargo, "usecase").dependencies.push({
        name: "adapter-platform-macos",
        kind: null,
        target,
        rename: null,
        source: null,
        optional: false,
        path: resolve(root, "packages/adapter-platform-macos"),
      });
      expect(f.check).toThrow("forbidden dependency");
    },
  );

  it.each([
    { source: "registry+https://github.com/rust-lang/crates.io-index", path: undefined },
    { source: null, path: resolve(root, "packages/port-platform") },
    { rename: "rules" },
    { optional: true },
  ] satisfies Partial<CargoDependency>[])(
    "rejects local source, alias and optional-edge escapes: %j",
    (change) => {
      const f = fixture();
      const dependency = member(f.cargo, "usecase").dependencies.find(
        (entry) => entry.name === "domain",
      )!;
      Object.assign(dependency, change);
      expect(f.check).toThrow(/local path|aliases|optional/u);
    },
  );

  it("rejects missing local dependencies", () => {
    const f = fixture();
    const owner = member(f.cargo, "usecase");
    owner.dependencies = owner.dependencies.filter((entry) => entry.name !== "domain");
    expect(f.check).toThrow("missing declared dependency");
  });

  it.each([null, ["crates-io"]])("rejects effective Cargo publication: %j", (publish) => {
    const f = fixture();
    member(f.cargo, "domain").publish = publish;
    expect(f.check).toThrow("publish must effectively be false");
  });

  it("rejects pnpm publication independently of its discovered private flag", () => {
    const f = fixture();
    f.jsManifests.get("apps/desktop")!.private = false;
    expect(f.check).toThrow("pnpm private must be true");
  });

  it("requires all consumers to declare the shared configuration dependency", () => {
    for (const directory of [
      ".",
      "apps/desktop",
      "packages/adapter-mcp-apps-host",
      "packages/adapter-mcp-apps-view",
      "packages/adapter-math-renderer",
    ]) {
      const f = fixture();
      delete f.jsManifests.get(directory)!.devDependencies!["typescript-config"];
      expect(f.check).toThrow("missing declared pnpm dependency");
    }
  });

  it.each([
    ["packages/adapter-mcp-apps-host", "adapter-math-renderer"],
    ["packages/adapter-mcp-apps-host", "adapter-mcp-apps-view"],
    ["packages/adapter-mcp-apps-view", "adapter-mcp-apps-host"],
    ["packages/adapter-math-renderer", "desktop"],
  ])("rejects a reverse or composition-only edge %s -> %s", (directory, dependency) => {
    const f = fixture();
    const manifest = f.jsManifests.get(directory)!;
    manifest.dependencies = { ...manifest.dependencies, [dependency]: "workspace:*" };
    expect(f.check).toThrow("unclassified pnpm local dependency or alias");
  });
  it("requires the explicit View-to-rich-content runtime edge", () => {
    const f = fixture();
    delete f.jsManifests.get("packages/adapter-mcp-apps-view")!.dependencies![
      "adapter-math-renderer"
    ];
    expect(f.check).toThrow("missing declared pnpm dependency");
  });

  it.each(["*", "file:../typescript-config", "npm:typescript-config@0.1.0"])(
    "rejects configuration dependency substitution: %s",
    (version) => {
      const f = fixture();
      f.jsManifests.get("apps/desktop")!.devDependencies!["typescript-config"] = version;
      expect(f.check).toThrow("unclassified pnpm local dependency or alias");
    },
  );

  it("rejects shared configuration as a runtime dependency or a reverse edge", () => {
    const f = fixture();
    f.jsManifests.get("apps/desktop")!.dependencies!["typescript-config"] = "workspace:*";
    expect(f.check).toThrow("unclassified pnpm local dependency or alias");
    const g = fixture();
    g.jsManifests.get("packages/typescript-config")!.devDependencies = { desktop: "workspace:*" };
    expect(g.check).toThrow("unclassified pnpm local dependency or alias");
  });

  it.each(["workspace:*", "file:../desktop", "link:../desktop", "npm:desktop@1"])(
    "rejects unclassified pnpm edge %s",
    (version) => {
      const f = fixture();
      f.jsManifests.get(".")!.devDependencies = { alias: version };
      expect(f.check).toThrow("unclassified pnpm local dependency or alias");
    },
  );

  it.each(["packages/domain/package.json", "packages/new/Cargo.toml", "apps/web/package.json"])(
    "rejects undiscovered or mixed-language manifests: %s",
    (path) => {
      const f = fixture();
      f.paths.push(path);
      expect(f.check).toThrow("Unclassified manifest");
    },
  );

  it("rejects unknown package admission into the development pnpm workspace", () => {
    const f = fixture();
    f.js.push({
      name: "unknown-package",
      path: resolve(root, "packages/unknown-package"),
      private: true,
    });
    expect(f.check).toThrow("Unclassified or missing pnpm member");
  });

  it("rejects Cargo packages missing from workspace_members", () => {
    const f = fixture();
    f.cargo.workspace_members.pop();
    expect(f.check).toThrow("exactly the workspace packages");
  });

  it("rejects duplicate pnpm names and package-path substitution", () => {
    const f = fixture();
    f.js[1]!.name = f.js[0]!.name;
    expect(f.check).toThrow("duplicate identity");
    const g = fixture();
    member(g.cargo, "domain").manifest_path = resolve(root, "packages/usecase/Cargo.toml");
    expect(g.check).toThrow("identity/path mismatch");
  });

  it("rejects feature declarations until their variants are reviewed", () => {
    const f = fixture();
    member(f.cargo, "domain").features = { native: [] };
    expect(f.check).toThrow("unreviewed package feature");
  });

  it.each([
    {},
    { "native-webview-lifecycle-test": [], extra: [] },
    { "native-webview-lifecycle-test": [], default: ["native-webview-lifecycle-test"] },
    { "native-webview-lifecycle-test": ["tauri/custom-protocol"] },
  ])("rejects drift from the reviewed opt-in lifecycle feature: %j", (features) => {
    const f = fixture();
    member(f.cargo, "desktop").features = features;
    expect(f.check).toThrow("unreviewed package feature");
  });

  it("rejects a portable build script, cross-package target and std module collision", () => {
    const f = fixture();
    member(f.cargo, "domain").targets.push({
      name: "build-script-build",
      kind: ["custom-build"],
      src_path: resolve(root, "packages/domain/build.rs"),
    });
    expect(f.check).toThrow("portable build script");
    const g = fixture();
    member(g.cargo, "domain").targets[0]!.src_path = resolve(root, "packages/usecase/src/lib.rs");
    expect(g.check).toThrow("cross-package target");
    const h = fixture();
    member(h.cargo, "domain").targets[0]!.name = "std";
    expect(h.check).toThrow("target name collision");
  });

  it("does not let failed or incomplete discovery become an empty success", () => {
    const f = fixture();
    f.cargo.packages = [];
    f.cargo.workspace_members = [];
    expect(f.check).toThrow("Unclassified or missing Cargo member");
    expect(() => repositoryPath(root, "../outside")).toThrow("escapes repository");
  });
});
