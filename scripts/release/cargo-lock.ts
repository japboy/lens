import { execFileSync } from "node:child_process";
import { readFileSync, realpathSync } from "node:fs";
import { relative, resolve } from "node:path";

type Package = { id: string; name: string; source: string | null; manifest_path: string };
type Metadata = {
  packages: Package[];
  workspace_members: string[];
  workspace_default_members: string[];
  resolve: {
    root: string | null;
    nodes: {
      id: string;
      dependencies: string[];
      deps: { name: string; pkg: string; dep_kinds: unknown[] }[];
      features: string[];
    }[];
  } | null;
};

function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) =>
    item && typeof item === "object" && !Array.isArray(item)
      ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b)))
      : item,
  );
}

function sorted<T>(values: T[]): T[] {
  return values.sort((a, b) => canonical(a).localeCompare(canonical(b)));
}

function cargo(root: string, args: string[], offline: boolean): string {
  return execFileSync("cargo", [...args, ...(offline ? ["--offline"] : [])], {
    cwd: root,
    encoding: "utf8",
    timeout: 120_000,
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function metadata(root: string, offline: boolean): Metadata {
  return JSON.parse(
    cargo(root, ["metadata", "--locked", "--all-features", "--format-version", "1"], offline),
  ) as Metadata;
}

function graphState(data: Metadata, root: string) {
  if (!data.resolve || !data.workspace_members.length) throw new Error("Missing Cargo resolution");
  const packages = new Map(data.packages.map((pkg) => [pkg.id, pkg]));
  if (packages.size !== data.packages.length) throw new Error("Duplicate Cargo package identity");
  const members = new Map<string, string>();
  for (const id of data.workspace_members) {
    const pkg = packages.get(id);
    if (!pkg || pkg.source !== null) throw new Error("Invalid Cargo workspace identity");
    const path = relative(realpathSync(root), realpathSync(pkg.manifest_path));
    if (path.startsWith("../") || path === "..") throw new Error("Workspace member outside root");
    members.set(id, canonical({ workspace: pkg.name, manifest: path }));
  }
  if (new Set(members.values()).size !== members.size)
    throw new Error("Duplicate Cargo workspace identity");
  const identity = (id: string) => {
    if (!packages.has(id)) throw new Error("Unknown Cargo resolution identity");
    return members.get(id) ?? id;
  };
  return {
    members: sorted([...members.values()]),
    defaultMembers: sorted(data.workspace_default_members.map(identity)),
    externalPackages: sorted(data.packages.filter((pkg) => !members.has(pkg.id))),
    root: data.resolve.root === null ? null : identity(data.resolve.root),
    nodes: sorted(
      data.resolve.nodes.map((node) => ({
        ...node,
        id: identity(node.id),
        dependencies: sorted(node.dependencies.map(identity)),
        deps: sorted(
          node.deps.map((dep) => ({
            ...dep,
            pkg: identity(dep.pkg),
            dep_kinds: sorted([...dep.dep_kinds]),
          })),
        ),
        features: sorted([...node.features]),
      })),
    ),
  };
}

// Cargo generates quoted, single-line identity fields. Reject ambiguous source records.
function sourcedInventory(lock: string): string[] {
  return sorted(
    lock
      .split(/^\[\[package\]\]\s*$/mu)
      .slice(1)
      .flatMap((entry) => {
        const field = (name: string) => {
          const matches = [...entry.matchAll(new RegExp(`^${name} = ("[^\\n]*")$`, "gmu"))];
          if (matches.length > 1) throw new Error("Ambiguous Cargo lock identity");
          return matches.length ? (JSON.parse(matches[0]![1]!) as string) : null;
        };
        const source = field("source");
        if (source === null) return [];
        const name = field("name");
        const version = field("version");
        if (!name || !version) throw new Error("Incomplete Cargo lock identity");
        return [canonical({ name, version, source, checksum: field("checksum") })];
      }),
  );
}

// Cargo owns lockfile reference spelling, including collisions with registry packages.
// The trusted baseline proves that this repair changes only workspace release identities.
export function refreshCargoLock(
  baselineRoot: string,
  candidateRoot: string,
  offline = false,
): string {
  if (realpathSync(baselineRoot) === realpathSync(candidateRoot))
    throw new Error("Cargo lock repair requires an isolated candidate");
  const baselineLock = readFileSync(resolve(baselineRoot, "Cargo.lock"), "utf8");
  const before = graphState(metadata(baselineRoot, offline), baselineRoot);
  cargo(candidateRoot, ["update", "--workspace"], offline);
  const after = graphState(metadata(candidateRoot, offline), candidateRoot);
  const candidateLock = readFileSync(resolve(candidateRoot, "Cargo.lock"), "utf8");
  if (canonical(before) !== canonical(after))
    throw new Error("Cargo lock repair changed dependency resolution");
  if (canonical(sourcedInventory(baselineLock)) !== canonical(sourcedInventory(candidateLock)))
    throw new Error("Cargo lock repair changed sourced package identities or checksums");
  return candidateLock;
}
