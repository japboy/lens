import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  graphArguments,
  inspectFeatureGraphs,
  parseFeatureGraph,
} from "./workspace-feature-graphs.ts";
import type { FeatureNode } from "./workspace-feature-graphs.ts";
import { BUILD_VARIANTS } from "./workspace-policy.ts";
import { assertBuildEnvironment, validateVariantAdmission } from "./run-workspace-variant.ts";

type Package = {
  name: string;
  version: string;
  source: string | null;
  dependencies: { name: string; kind: "dev" | "build" | null }[];
};
type Context = "host" | "target";
export type ContextNode = { context: Context; package: FeatureNode };
export type Projection = { nodes: ContextNode[]; edges: string[] };
const SHARED = ["domain", "port-platform", "use-case"];
const nodeKey = (node: ContextNode) => JSON.stringify(node);

// Actual common Tauri callers require Send for both exported async operations.
// Check their opaque return types as a consumer under the native feature projection,
// not just each producer's explicit return value. Never invoke platform effects.
export const NATIVE_FEATURE_CONSUMER = `
use std::sync::Arc;
use port_platform::{accessibility::Accessibility, capture::Capture};
use use_case::{context::{BlockingExecutor, ContextBuildRequest, build_context},
    confirm_targets::{ConfirmationHost, confirm_targets}};
fn requires_send<T: Send>(_: T) {}
pub fn check_async_consumers<E: BlockingExecutor, H: ConfirmationHost + Sync>(
    executor: &E, host: &H, accessibility: Arc<dyn Accessibility>, capture: Arc<dyn Capture>,
    request: ContextBuildRequest<'_>,
) {
    let operation = request.operation_id;
    requires_send(build_context(executor, accessibility, capture, request));
    requires_send(confirm_targets(host, operation));
}
`;

// Preserve occurrence paths from --no-dedupe, not a graph merged by package identity:
// an equal-feature parent can have different host/target child feature instances.
// Join each edge with metadata roles and reject ambiguous normal/build declarations.
export function sharedProductionProjection(
  output: string,
  root: string,
  packages: readonly Package[],
): Projection {
  const stack: (ContextNode & { active: boolean })[] = [];
  const roots = new Set<string>();
  const nodes = new Map<string, ContextNode>();
  const edges = new Set<string>();
  let sharedRoot = false;
  for (const line of output.trim().split("\n")) {
    if (!line) continue; // Cargo separates explicitly selected package roots with blank lines.
    const depthText = /^\d+/u.exec(line)?.[0];
    if (!depthText || line.endsWith(" (*)"))
      throw new Error(`Expected complete non-deduplicated production tree: ${line}`);
    const depth = Number(depthText);
    if (depth !== 0 && !sharedRoot) continue;
    const current = parseFeatureGraph(`0${line.slice(depthText.length)}`, root).nodes[0]!;
    if (depth === 0) {
      stack.length = 0;
      sharedRoot = SHARED.includes(current.name) && current.source.startsWith("path:");
      if (!sharedRoot) continue;
    }
    if (depth > stack.length) throw new Error("Incomplete production tree depth");
    const parent = depth ? stack[depth - 1] : undefined;
    let context: Context = current.source === "registry:proc-macro" ? "host" : "target";
    if (parent) {
      const metadata = packages.find(
        (entry) => entry.name === parent.package.name && entry.version === parent.package.version,
      );
      if (!metadata) throw new Error(`Missing package metadata: ${parent.package.name}`);
      const kinds = new Set(
        metadata.dependencies
          .filter((entry) => entry.name === current.name && entry.kind !== "dev")
          .map((entry) => entry.kind ?? "normal"),
      );
      if (kinds.size !== 1)
        throw new Error(`Ambiguous production edge: ${parent.package.name} -> ${current.name}`);
      context =
        parent.context === "host" || context === "host" || kinds.has("build") ? "host" : "target";
    }
    const shared = SHARED.includes(current.name) && current.source.startsWith("path:");
    if (shared && context === "target") roots.add(current.name);
    const active = shared || (parent?.active ?? false);
    const node = { context, package: current };
    if (active) nodes.set(nodeKey(node), node);
    if (parent?.active)
      edges.add(
        JSON.stringify([
          nodeKey({ context: parent.context, package: parent.package }),
          nodeKey(node),
        ]),
      );
    stack.length = depth;
    stack.push({ ...node, active });
  }
  if (roots.size !== SHARED.length) throw new Error("Incomplete shared production roots");
  return {
    nodes: [...nodes.entries()]
      .sort(([a], [b]) => a.localeCompare(b, "en"))
      .map(([, node]) => node),
    edges: [...edges].sort(),
  };
}

export function assertProductionProjection(expected: Projection, actual: Projection): void {
  if (JSON.stringify(expected) !== JSON.stringify(actual)) {
    const expectedNodes = expected.nodes.map(nodeKey),
      actualNodes = actual.nodes.map(nodeKey);
    throw new Error(
      `Native production feature projection differs: ${JSON.stringify({
        missing: expectedNodes.filter((entry) => !actualNodes.includes(entry)),
        extra: actualNodes.filter((entry) => !expectedNodes.includes(entry)),
        edgesEqual: JSON.stringify(expected.edges) === JSON.stringify(actual.edges),
      })}`,
    );
  }
}

export function verificationManifest(
  root: string,
  projection: Projection,
  rootManifest: string,
): string {
  const aliases = new Map(
    [...new Set(projection.nodes.map((node) => `${node.package.name}@${node.package.version}`))]
      .sort()
      .map((key, index) => [key, `dependency-${index}`]),
  );
  const lines = [
    "[package]",
    'name = "portable-native-verification"',
    'version = "0.0.0"',
    'edition = "2021"',
    "publish = false",
    "[workspace]",
    'resolver = "2"',
    "[lib]",
    'path = "lib.rs"',
  ];
  for (const context of ["target", "host"] as const) {
    lines.push(context === "target" ? "[dependencies]" : "[build-dependencies]");
    const identities = new Set<string>();
    for (const node of projection.nodes) {
      if (node.context !== context) continue;
      const dependency = node.package;
      const key = `${dependency.name}@${dependency.version}`;
      if (identities.has(key))
        throw new Error(`Multiple feature instances in one ${context} context: ${key}`);
      identities.add(key);
      if (dependency.source.startsWith("path:")) {
        if (!SHARED.includes(dependency.name))
          throw new Error("Native SDK dependency in portable projection");
        lines.push(
          `${JSON.stringify(dependency.name)} = { path = ${JSON.stringify(resolve(root, dependency.source.slice(5)))}, default-features = false }`,
        );
      } else {
        if (!["registry", "registry:proc-macro"].includes(dependency.source))
          throw new Error("Unsupported verification dependency source");
        lines.push(
          `${JSON.stringify(aliases.get(key))} = { package = ${JSON.stringify(dependency.name)}, version = ${JSON.stringify(`=${dependency.version}`)}, default-features = false, features = ${JSON.stringify(dependency.features)} }`,
        );
      }
    }
  }
  // Copy profile tables from the actual workspace, including package/build overrides.
  // Cargo validates the resulting manifest; there is no second hand-maintained profile.
  lines.push(
    rootManifest
      .split(/(?=^\[)/mu)
      .filter((table) => table.startsWith("[profile."))
      .join(""),
  );
  return `${lines.join("\n")}\n`;
}

export function checkPortableNativeFeatures(root: string): void {
  assertBuildEnvironment(process.env);
  const report = inspectFeatureGraphs(
    root,
    BUILD_VARIANTS.filter((variant) =>
      ["macos-production-check", "macos-bundle-build"].includes(variant.id),
    ),
  );
  validateVariantAdmission(
    report,
    JSON.parse(readFileSync(join(root, "scripts/workspace-variants.json"), "utf8")),
  );
  const cargo = (args: string[], cwd: string) =>
    execFileSync("cargo", args, { cwd, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  const metadata = (cwd: string, offline: boolean): { packages: Package[] } =>
    JSON.parse(
      cargo(
        [
          "metadata",
          "--locked",
          ...(offline ? ["--offline"] : []),
          "--format-version",
          "1",
          "--filter-platform",
          "aarch64-apple-darwin",
        ],
        cwd,
      ),
    );
  const original = metadata(root, false);
  const nativeVariant = BUILD_VARIANTS.find((variant) => variant.id === "macos-production-check")!;
  const expected = sharedProductionProjection(
    cargo([...graphArguments(nativeVariant), "--no-dedupe"], root),
    root,
    original.packages,
  );
  const bundledVariant = BUILD_VARIANTS.find((variant) => variant.id === "macos-bundle-build")!;
  assertProductionProjection(
    expected,
    sharedProductionProjection(
      cargo([...graphArguments(bundledVariant), "--no-dedupe"], root),
      root,
      original.packages,
    ),
  );
  const fixture = realpathSync(mkdtempSync(join(tmpdir(), "lens-native-features-")));
  try {
    // Only the verification workspace is generated. Production packages, manifests,
    // dependency locks and workspace membership are read without modification.
    writeFileSync(
      join(fixture, "Cargo.toml"),
      verificationManifest(root, expected, readFileSync(join(root, "Cargo.toml"), "utf8")),
    );
    writeFileSync(join(fixture, "lib.rs"), NATIVE_FEATURE_CONSUMER);
    writeFileSync(join(fixture, "build.rs"), "fn main() {}\n");
    copyFileSync(join(root, "Cargo.lock"), join(fixture, "Cargo.lock"));
    cargo(["generate-lockfile", "--offline"], fixture);
    const generated = metadata(fixture, true);
    for (const dependency of generated.packages.filter((entry) => entry.source !== null)) {
      if (
        !original.packages.some(
          (entry) =>
            entry.name === dependency.name &&
            entry.version === dependency.version &&
            entry.source === dependency.source,
        )
      )
        throw new Error(
          `Verification introduced a registry dependency: ${dependency.name}@${dependency.version}`,
        );
    }
    const output = cargo(
      [
        "tree",
        "--locked",
        "--offline",
        "--target",
        "aarch64-apple-darwin",
        "--edges",
        "normal,build",
        "--prefix",
        "depth",
        "--format",
        "{p}|{f}",
        "--charset",
        "ascii",
        "--no-dedupe",
      ],
      fixture,
    );
    const lines = output.trim().split("\n");
    if (!lines[0]?.startsWith("0portable-native-verification v0.0.0 ("))
      throw new Error("Unexpected verification graph root");
    const descendants = lines
      .slice(1)
      .map((line) =>
        line.replace(/^\d+/u, (depth) => {
          if (Number(depth) < 1) throw new Error("Unexpected extra verification graph root");
          return String(Number(depth) - 1);
        }),
      )
      .join("\n");
    const actual = sharedProductionProjection(descendants, root, generated.packages);
    assertProductionProjection(expected, actual);
    for (const profile of ["dev", "release"]) {
      const args = [
        "check",
        "--locked",
        "--offline",
        "--target",
        "aarch64-apple-darwin",
        "--lib",
        "--profile",
        profile,
        "--package",
        "portable-native-verification",
        ...SHARED.flatMap((name) => ["--package", name]),
      ];
      process.stdout.write(
        `${JSON.stringify({ verification: "native-production-shared", host: report.host, target: "aarch64-apple-darwin", contextNodes: expected.nodes.length, args })}\n`,
      );
      execFileSync("cargo", args, {
        cwd: fixture,
        stdio: "inherit",
        env: {
          ...process.env,
          CARGO_TARGET_DIR: resolve(root, "target/native-feature-verification"),
        },
      });
    }
  } finally {
    rmSync(fixture, { recursive: true, force: true });
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length !== 2) throw new Error("No implicit feature-update mode is provided");
  checkPortableNativeFeatures(fileURLToPath(new URL("..", import.meta.url)));
}
