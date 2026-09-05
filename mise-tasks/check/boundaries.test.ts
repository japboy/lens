import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { inspectWorkspace, repositoryPath, validateInventory } from "./boundaries.ts";
import type { CargoDependency, CargoInventory, PnpmManifest, PnpmMember } from "./boundaries.ts";
import {
  portableSourceViolations,
  rustTokens,
  sourceInclusionViolations,
} from "../../scripts/rust-source-boundaries.ts";
import { BUILD_VARIANTS, MEMBERS, variantArguments } from "../../scripts/workspace-policy.ts";

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
    expect(baseline.cargo.packages).toHaveLength(5);
    expect(pnpm).toHaveLength(3);
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

  it("requires both consumers to declare the shared configuration dependency", () => {
    for (const directory of [".", "apps/desktop"]) {
      const f = fixture();
      delete f.jsManifests.get(directory)!.devDependencies!["typescript-config"];
      expect(f.check).toThrow("missing declared pnpm dependency");
    }
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

  it("rejects managed-runtime admission into the development pnpm workspace", () => {
    const f = fixture();
    f.js.push({
      name: "managed-runtime",
      path: resolve(root, "apps/desktop/src-tauri/agent-runtime/codex"),
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

describe("portable source escape restrictions", () => {
  it.each([
    '#[cfg(target_os = "macos")] fn rule() {}',
    '#[cfg(any(test, target_arch = "aarch64"))] fn rule() {}',
    '#[cfg_attr(test, path = "../native.rs")] mod native;',
    'extern "C" { fn native(); }',
    "unsafe { native(); }",
    'std /* nested /* comment */ still comment */ :: fs::read("input");',
    "use std::{fs as disk};",
    "use std::r#process::Command;",
    "use std::time::Instant;",
    "let id = uuid::Uuid::new_v4();",
    'include!(concat!(env!("OUT_DIR"), "/native.rs"));',
    "macro_rules! inject { () => { native() } }",
    "#[unreviewed::expand] fn rule() {}",
    "unknown_macro!();",
    "windows::Win32::native();",
  ])("rejects %s", (source) => {
    expect(portableSourceViolations(source).length).toBeGreaterThan(0);
  });

  it("keeps strings, chars, lifetimes, raw strings and nested comments distinct from code", () => {
    const source = `/* outer /* unsafe */ extern */
      fn echo<'a>(value: &'a str) -> &'a str { value }
      let quote = '\\'';
      let escaped = "\\" unsafe extern fs";
      let raw = br##" /* unsafe */ " # "##;
      let windows = vec![1];
      #[cfg(test)] mod tests { #[test] fn works() { assert!(true); } }
    `;
    expect(portableSourceViolations(source)).toEqual([]);
    expect(rustTokens(source).some((token) => !token.literal && token.text === "unsafe")).toBe(
      false,
    );
  });

  it.each(['r##"unfinished"#', '"unfinished', "/* unterminated", "fn \u65e5\u672c\u8a9e() {}"])(
    "fails closed on unsupported or incomplete lexical input: %s",
    (source) => {
      expect(() => portableSourceViolations(source)).toThrow(/Unterminated|Non-ASCII/u);
    },
  );

  it("admits only the two root license documents as desktop text resources", () => {
    const check = (source: string) =>
      sourceInclusionViolations(
        root,
        "apps/desktop/src-tauri/src/about.rs",
        source,
        "apps/desktop",
      );
    expect(check('include_str!("../../../../LICENSE");')).toEqual([]);
    expect(check('include_str!("../../../../NOTICE");')).toEqual([]);
    expect(check('include_str!("../../../../Cargo.toml");')).not.toEqual([]);
    expect(check('include_bytes!("../../../../LICENSE");')).not.toEqual([]);
    expect(
      sourceInclusionViolations(
        root,
        "packages/domain/src/lib.rs",
        'include_str!("../../../LICENSE");',
        "packages/domain",
      ),
    ).not.toEqual([]);
  });

  it("rejects cross-owner and computed source/resource inclusion", () => {
    const check = (source: string) =>
      sourceInclusionViolations(root, "packages/domain/src/lib.rs", source, "packages/domain");
    for (const source of [
      'include!("own.rs");',
      'include_str!("../../usecase/src/model.rs");',
      'include_bytes!(concat!("../", "resource"));',
      '#[path = "../../usecase/src/model.rs"] mod other;',
      '#[cfg_attr(test, path = "../../usecase/src/model.rs")] mod other;',
    ])
      expect(check(source).length).toBeGreaterThan(0);
    expect(check('include_str!("../resource.txt");')).toEqual([]);
    expect(check('#[path = "other.rs"] mod other;')).toEqual([]);
    expect(portableSourceViolations('#[path = "other.rs"] mod other;')).not.toEqual([]);
  });
});

describe("explicit production and test variants", () => {
  it("uses finite exact target/package/feature/profile commands without workspace-wide Linux selection", () => {
    expect(new Set(BUILD_VARIANTS.map((variant) => variant.id)).size).toBe(BUILD_VARIANTS.length);
    for (const variant of BUILD_VARIANTS) {
      const args = variantArguments(variant);
      expect(args).toContain("--locked");
      expect(args).not.toContain("--workspace");
      expect(args).not.toContain("--all-targets");
      expect(variant.profile === "test").toBe(variant.operation === "test");
    }
    for (const variant of BUILD_VARIANTS.filter(
      (entry) => entry.target === "x86_64-unknown-linux-gnu",
    )) {
      expect(variant.packages).toContain("desktop");
      expect(variant.packages).not.toContain("adapter-platform-macos");
    }
    expect(BUILD_VARIANTS.some((variant) => variant.profile === "release")).toBe(true);
  });
});
