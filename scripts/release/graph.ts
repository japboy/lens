import type { FeatureGraph } from "../../mise-tasks/inspect/features.ts";
import { stableVersion } from "./version.ts";

// Only the exact application identity owns this slot. Dependency versions stay literal.
export function admissionGraph(graph: FeatureGraph, version: string): FeatureGraph {
  stableVersion(version);
  const projected = graph.nodes.map((node) => {
    if (node.name !== "desktop" || node.source !== "path:apps/desktop/src-tauri") return node;
    if (node.version !== version)
      throw new Error("Application graph version disagrees with authority");
    return { ...node, version: "application-release" };
  });
  const keys = projected.map((node) => JSON.stringify(node));
  if (new Set(keys).size !== keys.length)
    throw new Error("Projection would merge distinct graph nodes");
  const sorted = [...keys].sort();
  const index = new Map(sorted.map((key, position) => [key, position]));
  const map = (old: number) => {
    const key = keys[old];
    if (!key || !index.has(key)) throw new Error("Invalid graph node reference");
    return index.get(key)!;
  };
  return {
    nodes: sorted.map((key) => projected[keys.indexOf(key)]!),
    edges: graph.edges
      .map(([from, to]) => [keys[from], keys[to]] as const)
      .sort((a, b) =>
        JSON.stringify(a) < JSON.stringify(b) ? -1 : JSON.stringify(a) > JSON.stringify(b) ? 1 : 0,
      )
      .map(([from, to]) => {
        if (!from || !to) throw new Error("Invalid graph edge");
        return [index.get(from)!, index.get(to)!];
      }),
    roots: graph.roots.map(map).sort((a, b) => a - b),
  };
}
