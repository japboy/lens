import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { BUILD_VARIANTS } from "./workspace-policy.ts";
import type { BuildVariant } from "./workspace-policy.ts";
import { repositoryPath } from "./check-workspace-boundaries.ts";

export type FeatureNode = { name: string; version: string; source: string; features: string[] };
export type FeatureGraph = { nodes: FeatureNode[]; edges: [number, number][]; roots: number[] };

function identity(node: FeatureNode): string {
  return JSON.stringify(node);
}

// Preserve multiple feature instances of the same package (including build/proc-macro
// contexts). Do not flatten all versions/features into one workspace-wide union.
export function parseFeatureGraph(output: string, root: string): FeatureGraph {
  const nodes = new Map<string, FeatureNode>();
  const edges = new Set<string>();
  const roots = new Set<string>();
  const stack: string[] = [];
  for (const line of output.trim().split("\n")) {
    if (!line) continue;
    const match = /^(\d+)([A-Za-z0-9_-]+) v([^ |]+)(?: \(([^)]+)\))?\|([^ ]*)(?: \(\*\))?$/u.exec(
      line,
    );
    if (!match) throw new Error(`Unsupported cargo tree output: ${line}`);
    const depth = Number(match[1]);
    if (depth > stack.length) throw new Error(`Incomplete cargo tree depth: ${line}`);
    if (match[4] && match[4] !== "proc-macro" && !isAbsolute(match[4])) {
      throw new Error(`Unsupported dependency source: ${line}`);
    }
    const source = !match[4]
      ? "registry"
      : match[4] === "proc-macro"
        ? "registry:proc-macro"
        : `path:${repositoryPath(root, match[4])}`;
    const features = match[5] ? match[5].split(",").sort() : [];
    if (new Set(features).size !== features.length) throw new Error(`Duplicate feature: ${line}`);
    const node: FeatureNode = { name: match[2]!, version: match[3]!, source, features };
    const key = identity(node);
    if (line.endsWith(" (*)") && !nodes.has(key))
      throw new Error(`Incomplete repeated dependency: ${line}`);
    nodes.set(key, node);
    if (depth === 0) roots.add(key);
    else edges.add(JSON.stringify([stack[depth - 1], key]));
    stack.length = depth;
    stack.push(key);
  }
  if (!nodes.size || !roots.size) throw new Error("Empty cargo tree is not successful analysis");
  const keys = [...nodes.keys()].sort();
  const indices = new Map(keys.map((key, index) => [key, index]));
  return {
    nodes: keys.map((key) => nodes.get(key)!),
    edges: [...edges].sort().map((edge) => {
      const [from, to] = JSON.parse(edge) as [string, string];
      return [indices.get(from)!, indices.get(to)!];
    }),
    roots: [...roots].sort().map((key) => indices.get(key)!),
  };
}

export function graphDigest(graph: FeatureGraph): string {
  return createHash("sha256").update(JSON.stringify(graph)).digest("hex");
}

export function assertGraphMatches(expected: FeatureGraph, actual: FeatureGraph): void {
  if (graphDigest(expected) !== graphDigest(actual))
    throw new Error("Dependency/feature graph changed; native variant review required");
}

export function graphArguments(variant: BuildVariant): string[] {
  return [
    "tree",
    "--locked",
    "--target",
    variant.target,
    ...variant.packages.flatMap((name) => ["--package", name]),
    "--edges",
    variant.operation === "test" ? "normal,build,dev" : "normal,build",
    "--prefix",
    "depth",
    "--format",
    "{p}|{f}",
    "--charset",
    "ascii",
    ...(!variant.defaultFeatures ? ["--no-default-features"] : []),
    ...(variant.features.length ? ["--features", variant.features.join(",")] : []),
  ];
}

export function inspectFeatureGraphs(
  root: string,
  selection: readonly BuildVariant[] = BUILD_VARIANTS,
) {
  const rustc = execFileSync("rustc", ["-vV"], { cwd: root, encoding: "utf8" });
  const host = /^host: (.+)$/mu.exec(rustc)?.[1];
  if (!host || !["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"].includes(host))
    throw new Error("Unreviewed graph-analysis host");
  const variants = selection.map((variant) => {
    const args = graphArguments(variant);
    const output = execFileSync("cargo", args, {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
    });
    const graph = parseFeatureGraph(output, root);
    const discoveredRoots = graph.roots.map((index) => graph.nodes[index]!.name).sort();
    if (JSON.stringify(discoveredRoots) !== JSON.stringify([...variant.packages].sort()))
      throw new Error(`${variant.id}: incomplete selected root graph`);
    if (variant.packages.includes("desktop")) {
      const tauri = graph.nodes.filter((node) => node.name === "tauri");
      if (
        !tauri.length ||
        tauri.some((node) => node.features.includes("test") !== (variant.operation === "test"))
      )
        throw new Error(`${variant.id}: normal/test Tauri feature isolation failed`);
    } else if (
      graph.nodes.some((node) => node.name === "tauri" || node.name === "adapter-platform-macos")
    ) {
      throw new Error(`${variant.id}: portable transitive graph contains native shell`);
    }
    return {
      variant: variant.id,
      args,
      profile: variant.profile,
      digest: graphDigest(graph),
      graph,
    };
  });
  // This is reviewable resolution evidence, not compiler-unit or CI-skip attestation.
  // Profile-specific normal compilation, both host snapshots and admission remain required.
  return { version: 1, host, rustc, variants };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length !== 2)
    throw new Error("No implicit graph update or admission mode is provided");
  const report = inspectFeatureGraphs(fileURLToPath(new URL("..", import.meta.url)));
  process.stdout.write(
    `${JSON.stringify({ ...report, variants: report.variants.map(({ graph, ...variant }) => ({ ...variant, nodes: graph.nodes.length, edges: graph.edges.length })) }, null, 2)}\n`,
  );
}
