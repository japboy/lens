import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
export type WorkspaceMember = { directory: string };
export type SourceSnapshotContract = {
  repository: string;
  applicationPath: string;
  members: readonly WorkspaceMember[];
  inputs: readonly string[];
};
/** Dependency closure is declared by workspace metadata and package manifests. */
export function workspaceDirectories(contract: SourceSnapshotContract): string[] {
  const members = contract.members.map(({ directory }) => ({
    directory,
    manifest: JSON.parse(
      readFileSync(join(contract.repository, directory, "package.json"), "utf8"),
    ) as {
      name: string;
      dependencies?: Record<string, string>;
      devDependencies?: Record<string, string>;
    },
  }));
  if (
    new Set(members.map((member) => member.manifest.name)).size !== members.length ||
    new Set(members.map((member) => member.directory)).size !== members.length
  )
    throw new Error("Duplicate workspace member identity");
  const byName = new Map(members.map((member) => [member.manifest.name, member]));
  const byDirectory = new Map(members.map((member) => [member.directory, member]));
  const selected = new Set<string>();
  function visit(directory: string): void {
    if (selected.has(directory)) return;
    const member = byDirectory.get(directory);
    if (!member) throw new Error(`Unknown workspace member: ${directory}`);
    selected.add(directory);
    for (const [name, version] of Object.entries({
      ...member.manifest.dependencies,
      ...member.manifest.devDependencies,
    })) {
      const dependency = byName.get(name);
      if (dependency) {
        if (version !== "workspace:*") throw new Error(`Unsealed local dependency: ${name}`);
        visit(dependency.directory);
      } else if (/^(?:workspace:|file:|link:)/u.test(version))
        throw new Error(`Unknown local dependency: ${name}`);
    }
  }
  visit(".");
  visit(contract.applicationPath);
  return [...selected].sort();
}
export function sourcePaths(contract: SourceSnapshotContract): string[] {
  return [
    ...new Set([
      ...contract.inputs,
      "package.json",
      `${contract.applicationPath}/package.json`,
      ...workspaceDirectories(contract).filter(
        (directory) => directory !== "." && directory !== contract.applicationPath,
      ),
    ]),
  ].sort();
}
export function sourceInputs(contract: SourceSnapshotContract): Map<string, Buffer> {
  const paths = sourcePaths(contract);
  const execute = (args: string[]) =>
    execFileSync("git", ["ls-files", "-z", ...args, "--", ...paths], {
      cwd: contract.repository,
      encoding: "utf8",
    })
      .split("\0")
      .filter(Boolean);
  const deleted = new Set(execute(["--deleted"]));
  return new Map(
    [...new Set(execute(["--cached", "--others", "--exclude-standard"]))]
      .filter((file) => !deleted.has(file))
      .sort()
      .map((file) => [file, readFileSync(join(contract.repository, file))]),
  );
}
export function sourceDigest(files: Map<string, Buffer>): string {
  const hash = createHash("sha256");
  for (const [file, bytes] of [...files].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
    hash.update(file).update("\0").update(bytes).update("\0");
  return hash.digest("hex");
}
