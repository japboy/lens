#!/usr/bin/env node
//MISE description = "Check workspace identities, dependency roles and Rust source boundaries"
//MISE dir = "{{config_root}}"

import { execFileSync } from "node:child_process";
import { existsSync, lstatSync, readFileSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { repositoryFiles } from "../../scripts/repository-files.ts";
import {
  DEPENDENCY_FEATURES,
  MEMBERS,
  TARGET_DEPENDENCIES,
} from "../../scripts/workspace-policy.ts";
import type { DependencyKind, Member } from "../../scripts/workspace-policy.ts";
import {
  portableSourceViolations,
  sourceInclusionViolations,
} from "../../scripts/rust-source-boundaries.ts";
import {
  commonShellConditionalViolations,
  nativeCompositionViolations,
  rustDeclarationSurface,
} from "../../scripts/rust-source-surface.ts";

export type CargoDependency = {
  name: string;
  kind: "dev" | "build" | null;
  target: string | null;
  rename: string | null;
  source: string | null;
  path?: string;
  optional: boolean;
  features: string[];
  uses_default_features: boolean;
};
export type CargoPackage = {
  id: string;
  name: string;
  manifest_path: string;
  source: string | null;
  publish: string[] | null;
  dependencies: CargoDependency[];
  features: Record<string, unknown>;
  targets: { name: string; kind: string[]; src_path: string }[];
};
export type CargoInventory = {
  workspace_root: string;
  workspace_members: string[];
  packages: CargoPackage[];
};
export type PnpmMember = { name: string; path: string; private?: boolean };
export type PnpmManifest = {
  name: string;
  private?: boolean;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
};

const RESERVED_RUST_NAMES = new Set([
  "std",
  "core",
  "alloc",
  "crate",
  "self",
  "super",
  "test",
  "proc_macro",
  "any",
  "array",
  "ascii",
  "borrow",
  "boxed",
  "cell",
  "char",
  "clone",
  "cmp",
  "collections",
  "convert",
  "default",
  "env",
  "error",
  "ffi",
  "fmt",
  "fs",
  "future",
  "hash",
  "hint",
  "io",
  "iter",
  "marker",
  "mem",
  "net",
  "num",
  "ops",
  "option",
  "os",
  "panic",
  "path",
  "pin",
  "prelude",
  "primitive",
  "process",
  "ptr",
  "rc",
  "result",
  "slice",
  "str",
  "string",
  "sync",
  "task",
  "thread",
  "time",
  "vec",
]);

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function unique(values: readonly string[], context: string): void {
  assert(new Set(values).size === values.length, `${context}: duplicate identity`);
}

export function repositoryPath(root: string, path: string): string {
  const local = relative(root, resolve(root, path)).replaceAll("\\", "/");
  assert(
    local !== ".." && !local.startsWith("../") && !isAbsolute(local),
    `Path escapes repository: ${path}`,
  );
  return local || ".";
}

export function validateInventory(
  root: string,
  cargo: CargoInventory,
  pnpm: readonly PnpmMember[],
  manifests: ReadonlyMap<string, PnpmManifest>,
  sourcePaths: readonly string[],
  policy: readonly Member[] = MEMBERS,
): void {
  unique(
    policy.map((member) => `${member.ecosystem}:${member.name}`),
    "policy names",
  );
  unique(
    policy.map((member) => `${member.ecosystem}:${member.directory}`),
    "policy directories",
  );
  assert(resolve(cargo.workspace_root) === resolve(root), "Unexpected Cargo workspace root");
  unique(cargo.workspace_members, "Cargo members");
  unique(
    cargo.packages.map((member) => member.id),
    "Cargo packages",
  );
  assert(
    cargo.workspace_members.length === cargo.packages.length,
    "Inventory must contain exactly the workspace packages",
  );
  assert(
    cargo.packages.every((member) => cargo.workspace_members.includes(member.id)),
    "Unclassified Cargo package",
  );
  unique(
    cargo.packages.map((member) => member.name),
    "Cargo names",
  );
  unique(
    pnpm.map((member) => member.name),
    "pnpm names",
  );
  unique(
    pnpm.map((member) => resolve(member.path)),
    "pnpm paths",
  );

  const expectedCargo = policy.filter((member) => member.ecosystem === "cargo");
  const expectedPnpm = policy.filter((member) => member.ecosystem === "pnpm");
  assert(expectedCargo.length === cargo.packages.length, "Unclassified or missing Cargo member");
  assert(expectedPnpm.length === pnpm.length, "Unclassified or missing pnpm member");
  const localCargo = new Map(expectedCargo.map((member) => [member.name, member]));

  for (const member of cargo.packages) {
    const owner = localCargo.get(member.name);
    assert(
      owner && repositoryPath(root, dirname(member.manifest_path)) === owner.directory,
      `Cargo identity/path mismatch: ${member.name}`,
    );
    assert(
      member.source === null && Array.isArray(member.publish) && member.publish.length === 0,
      `${member.name}: publish must effectively be false`,
    );
    assert(
      !RESERVED_RUST_NAMES.has(member.name.replaceAll("-", "_")),
      `${member.name}: standard module collision`,
    );
    assert(
      Object.keys(member.features).length === 0,
      `${member.name}: unreviewed package feature declaration`,
    );
    for (const target of member.targets) {
      const path = repositoryPath(root, target.src_path);
      assert(path.startsWith(`${owner.directory}/`), `${member.name}: cross-package target source`);
      if (target.kind.includes("custom-build")) {
        assert(
          owner.implementation === "common-shell" || owner.implementation === "macos",
          `${member.name}: portable build script`,
        );
      } else {
        assert(!RESERVED_RUST_NAMES.has(target.name), `${member.name}: target name collision`);
      }
    }
    const seen = new Set<string>();
    for (const dependency of member.dependencies) {
      const kind: DependencyKind = dependency.kind ?? "normal";
      assert(["normal", "dev", "build"].includes(kind), `${member.name}: unknown dependency kind`);
      const key = `${kind}:${dependency.target ?? "*"}:${dependency.name}`;
      assert(!seen.has(key), `${member.name}: duplicate dependency ${key}`);
      seen.add(key);
      assert(
        dependency.rename === null,
        `${member.name}: dependency aliases require policy review`,
      );
      assert(!dependency.optional, `${member.name}: optional dependency requires variant review`);
      const allowed =
        dependency.target === null
          ? owner.dependencies[kind]?.includes(dependency.name)
          : TARGET_DEPENDENCIES.some(
              (edge) =>
                edge.member === member.name &&
                edge.name === dependency.name &&
                edge.kind === kind &&
                edge.target === dependency.target,
            );
      assert(allowed, `${member.name}: forbidden dependency ${key}`);
      // A dependency's name says which crate is linked; its features say what that crate
      // is allowed to do. Pin both at the same granularity so widening a capability is a
      // reviewed policy change rather than an unnoticed manifest edit.
      const declared = DEPENDENCY_FEATURES.find(
        (entry) =>
          entry.member === member.name &&
          entry.name === dependency.name &&
          entry.kind === kind &&
          entry.target === dependency.target,
      );
      const features = [...dependency.features].sort();
      const expected = declared ? [...declared.features].sort() : [];
      assert(
        dependency.uses_default_features === (declared?.default ?? true) &&
          features.length === expected.length &&
          features.every((feature, index) => feature === expected[index]),
        `${member.name}: unreviewed feature selection for ${key}`,
      );
      const local = localCargo.get(dependency.name);
      if (local) {
        assert(
          dependency.source === null &&
            dependency.path !== undefined &&
            repositoryPath(root, dependency.path) === local.directory,
          `${member.name}: ${dependency.name} must use its exact local path`,
        );
      } else {
        assert(
          dependency.path === undefined &&
            dependency.source === "registry+https://github.com/rust-lang/crates.io-index",
          `${member.name}: unclassified dependency source ${dependency.name}`,
        );
      }
    }
    const expected = Object.entries(owner.dependencies).flatMap(([kind, names]) =>
      names.map((name) => `${kind}:*:${name}`),
    );
    expected.push(
      ...TARGET_DEPENDENCIES.filter((edge) => edge.member === member.name).map(
        (edge) => `${edge.kind}:${edge.target}:${edge.name}`,
      ),
    );
    assert(
      expected.length === seen.size && expected.every((edge) => seen.has(edge)),
      `${member.name}: missing declared dependency`,
    );
  }

  for (const member of pnpm) {
    const owner = expectedPnpm.find((entry) => entry.name === member.name);
    assert(
      owner && repositoryPath(root, member.path) === owner.directory,
      `pnpm identity/path mismatch: ${member.name}`,
    );
    const manifest = manifests.get(owner.directory);
    assert(
      member.private === true && manifest?.private === true && manifest.name === member.name,
      `${member.name}: pnpm private must be true`,
    );
    const seen = new Set<string>();
    for (const [kind, section] of [
      ["normal", manifest.dependencies],
      ["dev", manifest.devDependencies],
      ["optional", manifest.optionalDependencies],
      ["peer", manifest.peerDependencies],
    ] as const) {
      for (const [name, version] of Object.entries(section ?? {})) {
        if (
          expectedPnpm.some((entry) => entry.name === name) ||
          /^(?:workspace:|file:|link:|npm:)/u.test(version)
        ) {
          assert(
            (kind === "normal" || kind === "dev") &&
              owner.dependencies[kind]?.includes(name) &&
              version === "workspace:*",
            `${member.name}: unclassified pnpm local dependency or alias ${name}`,
          );
          seen.add(`${kind}:${name}`);
        }
      }
    }
    const expected = Object.entries(owner.dependencies).flatMap(([kind, names]) =>
      names.map((name) => `${kind}:${name}`),
    );
    assert(
      expected.length === seen.size && expected.every((edge) => seen.has(edge)),
      `${member.name}: missing declared pnpm dependency`,
    );
  }

  const admittedManifests = new Set([
    "Cargo.toml",
    ...expectedCargo.map((member) => `${member.directory}/Cargo.toml`),
    ...expectedPnpm.map((member) =>
      member.directory === "." ? "package.json" : `${member.directory}/package.json`,
    ),
  ]);
  for (const path of sourcePaths) {
    if (/(?:^|\/)(?:Cargo.toml|package.json)$/u.test(path)) {
      assert(admittedManifests.has(path), `Unclassified manifest: ${path}`);
    }
  }
  for (const path of admittedManifests)
    assert(sourcePaths.includes(path), `Missing manifest: ${path}`);
}

export function inspectWorkspace(root: string): { cargo: CargoInventory; paths: string[] } {
  // `repositoryFiles` rejects symbolic links, so every path here resolves inside the
  // repository; the containment check below still confirms that for each one.
  const paths = repositoryFiles(root);
  for (const path of paths) {
    repositoryPath(root, realpathSync(resolve(root, path)));
  }
  const cargo: CargoInventory = JSON.parse(
    execFileSync("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
    }),
  );
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
  validateInventory(root, cargo, pnpm, manifests, paths);
  for (const path of paths.filter((entry) => entry.endsWith(".rs"))) {
    const member = MEMBERS.find(
      (entry) => entry.ecosystem === "cargo" && path.startsWith(`${entry.directory}/`),
    );
    assert(member, `Unclassified Rust source: ${path}`);
    const source = readFileSync(resolve(root, path), "utf8");
    const owner = member.role === "application" ? "apps/desktop" : member.directory;
    const violations = sourceInclusionViolations(root, path, source, owner);
    if (member.implementation === "portable") {
      violations.push(...portableSourceViolations(source));
      rustDeclarationSurface(source);
    }
    if (path.startsWith("apps/desktop/src-tauri/src/native/"))
      violations.push(...nativeCompositionViolations(source));
    else if (path.startsWith("apps/desktop/src-tauri/src/"))
      violations.push(...commonShellConditionalViolations(path, source));
    assert(violations.length === 0, `${path}: ${violations.join("; ")}`);
  }
  return { cargo, paths };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const root = fileURLToPath(new URL("../../", import.meta.url));
  const { cargo, paths } = inspectWorkspace(root);
  process.stdout.write(
    `Workspace boundaries passed: ${cargo.packages.length} Cargo members, ${MEMBERS.filter((member) => member.ecosystem === "pnpm").length} pnpm members, ${paths.length} source paths.\n`,
  );
}
