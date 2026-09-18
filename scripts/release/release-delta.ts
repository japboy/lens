import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import { lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parse } from "@iarna/toml";
import { refreshCargoLock } from "./cargo-lock.ts";
import { changelogSection, commitSha } from "./source.ts";
import { compareVersions, manifestVersionState, VERSION_FILES, versionState } from "./version.ts";

export type ReleaseFile = { mode: "100644" | "100755"; content: Buffer };
export type ReleaseTree = ReadonlyMap<string, ReleaseFile>;
export const RELEASE_UPDATE_PATHS: ReadonlySet<string> = new Set([
  ...VERSION_FILES.filter((path) => !path.endsWith("/Cargo.toml")),
  "CHANGELOG.md",
]);

export function releasePath(path: string): string {
  if (
    !path ||
    path.includes("\\") ||
    path.includes("\0") ||
    path.split("/").some((part) => !part || part === "." || part === "..") ||
    path.startsWith("/")
  )
    throw new Error("Expected a contained repository path");
  return path;
}

function text(tree: ReleaseTree, path: string): string {
  const file = tree.get(path);
  if (!file || file.mode !== "100644") throw new Error(`Missing regular release file: ${path}`);
  const value = file.content.toString("utf8");
  if (!Buffer.from(value).equals(file.content))
    throw new Error(`Invalid UTF-8 release file: ${path}`);
  return value;
}

function versionFiles(tree: ReleaseTree): Record<string, string> {
  return Object.fromEntries(VERSION_FILES.map((path) => [path, text(tree, path)]));
}

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Expected a manifest object");
  return value as Record<string, unknown>;
}

function withoutVersion(path: string, content: string): unknown {
  if (path === "Cargo.toml") {
    const value = parse(content);
    delete object(object(value.workspace).package).version;
    return value;
  }
  const value = object(JSON.parse(content));
  delete value[path === ".release-please-manifest.json" ? "." : "version"];
  return value;
}

/** Semantic admission is relative to the exact base used by the required CI check. */
export function assertReleaseDelta(
  baseline: ReleaseTree,
  candidate: ReleaseTree,
  complete = true,
): { version: string; changedPaths: string[] } {
  const before = versionState(versionFiles(baseline));
  const proposed = (complete ? versionState : manifestVersionState)(versionFiles(candidate));
  if (
    !proposed.bootstrapped ||
    !(
      compareVersions(proposed.version, before.version) > 0 ||
      (!before.bootstrapped && proposed.version === "0.1.0")
    )
  )
    throw new Error("Release candidate must advance the product version or bootstrap 0.1.0");
  const changedPaths: string[] = [];
  for (const path of new Set([...baseline.keys(), ...candidate.keys()])) {
    releasePath(path);
    const old = baseline.get(path);
    const next = candidate.get(path);
    if (old && next && old.mode === next.mode && old.content.equals(next.content)) continue;
    if (
      !RELEASE_UPDATE_PATHS.has(path) ||
      !next ||
      next.mode !== "100644" ||
      (old && old.mode !== "100644") ||
      (!old && path !== "CHANGELOG.md")
    )
      throw new Error(`Release candidate changed an unowned path or mode: ${path}`);
    changedPaths.push(path);
    if (path === "Cargo.lock") continue; // Cargo graph and byte regeneration is checked separately.
    if (path === "CHANGELOG.md") {
      const current = text(candidate, path);
      changelogSection(current, proposed.version);
      if (old) {
        const previous = text(baseline, path);
        const oldStart = previous.search(/^## /mu);
        const newHeadings = [...current.matchAll(/^## /gmu)];
        if (
          oldStart < 0 ||
          newHeadings.length < 2 ||
          current.slice(newHeadings[1]!.index) !== previous.slice(oldStart) ||
          current.slice(0, newHeadings[0]!.index) !== previous.slice(0, oldStart)
        )
          throw new Error("Release candidate rewrote changelog history or preamble");
      }
      continue;
    }
    if (
      !isDeepStrictEqual(
        withoutVersion(path, text(baseline, path)),
        withoutVersion(path, text(candidate, path)),
      )
    )
      throw new Error(`Release candidate changed non-version manifest data: ${path}`);
  }
  if (!changedPaths.includes("CHANGELOG.md"))
    throw new Error("Release candidate must prepend its changelog section");
  return { version: proposed.version, changedPaths: changedPaths.sort() };
}

export type ReleaseSnapshot = { directory: string; files: Map<string, ReleaseFile> };
const MAX_SNAPSHOT_BYTES = 256 * 1024 * 1024;

/** Archive fixed commits without linked worktrees, hooks, checkout filters or source execution. */
export function snapshotReleaseSource(
  root: string,
  sha: string,
  directory: string,
): ReleaseSnapshot {
  commitSha(sha);
  const entries = execFileSync("git", ["ls-tree", "-rz", "--full-tree", sha], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: MAX_SNAPSHOT_BYTES,
    timeout: 120_000,
  })
    .split("\0")
    .filter(Boolean)
    .map((entry) => {
      const match = /^(100644|100755) blob ([a-f0-9]{40})\t(.+)$/su.exec(entry);
      if (!match)
        throw new Error(
          "Release snapshots require regular tracked files, without links or submodules",
        );
      return {
        mode: match[1] as ReleaseFile["mode"],
        oid: match[2]!,
        path: releasePath(match[3]!),
      };
    });
  mkdirSync(directory, { recursive: false });
  const archive = execFileSync("git", ["archive", "--format=tar", sha], {
    cwd: root,
    maxBuffer: MAX_SNAPSHOT_BYTES,
    timeout: 120_000,
  });
  // tar can stop reading at its end-of-archive blocks before stdin padding has
  // drained. A bounded regular file avoids treating that valid exit as EPIPE.
  const archiveDirectory = mkdtempSync(join(tmpdir(), "lens-release-archive-"));
  try {
    const archivePath = join(archiveDirectory, "source.tar");
    writeFileSync(archivePath, archive, { flag: "wx" });
    execFileSync("tar", ["-xf", archivePath, "-C", directory], {
      maxBuffer: MAX_SNAPSHOT_BYTES,
      timeout: 120_000,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } finally {
    rmSync(archiveDirectory, { recursive: true, force: true });
  }
  const files = new Map<string, ReleaseFile>();
  let size = 0;
  for (const entry of entries) {
    const path = join(directory, entry.path);
    const stat = lstatSync(path);
    if (!stat.isFile() || stat.isSymbolicLink())
      throw new Error("Archive did not produce regular files");
    size += stat.size;
    if (size > MAX_SNAPSHOT_BYTES) throw new Error("Release snapshot exceeds size limit");
    const content = readFileSync(path);
    const oid = createHash("sha1").update(`blob ${content.length}\0`).update(content).digest("hex");
    if (oid !== entry.oid) throw new Error("Archive content differs from the exact Git blob");
    files.set(entry.path, { mode: entry.mode, content });
  }
  return { directory, files };
}

/** No network or refs are mutated. Both snapshots are removed on success and failure. */
export function verifyReleaseDelta(
  root: string,
  baseSha: string,
  headSha: string,
  offline = false,
): { version: string; changedPaths: string[] } {
  const temporary = mkdtempSync(join(tmpdir(), "lens-release-delta-"));
  try {
    const baseline = snapshotReleaseSource(root, baseSha, join(temporary, "baseline"));
    const candidate = snapshotReleaseSource(root, headSha, join(temporary, "candidate"));
    const result = assertReleaseDelta(baseline.files, candidate.files);
    const original = text(candidate.files, "Cargo.lock");
    if (refreshCargoLock(baseline.directory, candidate.directory, offline) !== original)
      throw new Error("Release candidate Cargo.lock is not Cargo's exact workspace update");
    return result;
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}
