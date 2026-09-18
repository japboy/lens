import { cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { Errors, Manifest, registerPlugin } from "release-please";
import { ManifestPlugin } from "release-please/build/src/plugin.js";
import type { CandidateReleasePullRequest } from "release-please/build/src/manifest.js";
import type { Scm } from "release-please/build/src/scm.js";
import type { Update } from "release-please/build/src/update.js";
import { refreshCargoLock } from "./cargo-lock.ts";
import { installationNotes } from "./artifact.ts";
import {
  assertReleaseDelta,
  RELEASE_UPDATE_PATHS,
  releasePath,
  snapshotReleaseSource,
} from "./release-delta.ts";
import type { ReleaseFile, ReleaseSnapshot } from "./release-delta.ts";
import { commitSha } from "./source.ts";

const PLUGIN = "lens-cargo-workspace";
type Context = { baseline: ReleaseSnapshot; temporary: string; offline: boolean };
const contexts = new WeakMap<Scm, Context>();

class CargoProposalPlugin extends ManifestPlugin {
  override async run(
    candidates: CandidateReleasePullRequest[],
  ): Promise<CandidateReleasePullRequest[]> {
    if (!candidates.length) return candidates;
    if (candidates.length !== 1 || candidates[0]!.path !== ".")
      throw new Error("Expected one root release candidate");
    const context = contexts.get(this.github);
    if (!context) throw new Error("Missing isolated proposal context");
    const directory = mkdtempSync(join(context.temporary, "candidate-"));
    try {
      cpSync(context.baseline.directory, directory, { recursive: true });
      const files = new Map(context.baseline.files);
      const candidate = candidates[0]!;
      const paths = new Set<string>();
      // Compose repeated updates in their declared order, matching upstream mergeUpdates.
      for (const update of candidate.pullRequest.updates) {
        const path = releasePath(update.path);
        const previous = files.get(path);
        // Node strategy proposes optional ecosystem files even when absent.
        if (!previous && !update.createIfMissing) continue;
        if (
          !RELEASE_UPDATE_PATHS.has(path) ||
          path === "Cargo.lock" ||
          (!previous && path !== "CHANGELOG.md") ||
          (previous && previous.mode !== "100644")
        )
          throw new Error(`Unowned release updater: ${path}`);
        const content = update.updater.updateContent(
          previous?.content.toString("utf8"),
          this.logger,
        );
        if (typeof content !== "string" || !content)
          throw new Error(`Release updater returned empty content: ${path}`);
        if (Buffer.byteLength(content) > 16 * 1024 * 1024)
          throw new Error(`Release updater exceeded the file size limit: ${path}`);
        const file: ReleaseFile = { mode: "100644", content: Buffer.from(content) };
        files.set(path, file);
        paths.add(path);
        mkdirSync(dirname(join(directory, path)), { recursive: true });
        writeFileSync(join(directory, path), file.content);
      }
      // Reject source/feature/config changes before even invoking Cargo.
      assertReleaseDelta(context.baseline.files, files, false);
      const lock = refreshCargoLock(context.baseline.directory, directory, context.offline);
      files.set("Cargo.lock", { mode: "100644", content: Buffer.from(lock) });
      paths.add("Cargo.lock");
      const admitted = assertReleaseDelta(context.baseline.files, files);
      const releaseData = candidate.pullRequest.body.releaseData;
      if (releaseData.length !== 1) throw new Error("Expected one release notes entry");
      const guidance = installationNotes(admitted.version);
      const notes = releaseData[0]!.notes.trimEnd();
      if (notes.includes("## Install and update") && !notes.endsWith(guidance))
        throw new Error("Release notes already contain conflicting installation guidance");
      releaseData[0]!.notes = notes.endsWith(guidance) ? notes : `${notes}\n\n${guidance}`;
      candidate.pullRequest.updates = [...paths].sort().map((path): Update => ({
        path,
        createIfMissing: path === "CHANGELOG.md",
        updater: { updateContent: () => files.get(path)!.content.toString("utf8") },
      }));
      return candidates;
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }
}

registerPlugin(
  PLUGIN,
  (options) =>
    new CargoProposalPlugin(options.github, options.targetBranch, options.repositoryConfig),
);

export type ProposalOptions = { github: Scm; root: string; baseSha: string; offline?: boolean };

/** Scope all local resources and fixed-base readers to one invocation. */
export async function withReleaseProposalManifest<T>(
  options: ProposalOptions,
  operation: (manifest: Manifest) => Promise<T>,
): Promise<T> {
  const sha = commitSha(options.baseSha);
  const temporary = mkdtempSync(join(tmpdir(), "lens-release-proposal-"));
  let scoped: Scm | undefined;
  try {
    const baseline = snapshotReleaseSource(options.root, sha, join(temporary, "baseline"));
    const file = (path: string) => {
      const entry = baseline.files.get(releasePath(path));
      if (!entry) throw new Errors.FileNotFoundError(path);
      if (entry.mode !== "100644")
        throw new Error("Release Please may read only regular manifest files");
      return {
        content: entry.content.toString("base64"),
        parsedContent: entry.content.toString("utf8"),
        mode: entry.mode,
        sha: createHash("sha1")
          .update(`blob ${entry.content.length}\0`)
          .update(entry.content)
          .digest("hex"),
        path,
      };
    };
    const overrides: Partial<Scm> = {
      getFileContents: async (path) => file(path),
      getFileContentsOnBranch: async (path, branch) =>
        branch === "main" ? file(path) : options.github.getFileContentsOnBranch(path, branch),
      getFileJson: async <Value>(path: string, branch: string): Promise<Value> =>
        branch === "main"
          ? (JSON.parse(file(path).parsedContent) as Value)
          : options.github.getFileJson<Value>(path, branch),
      async *mergeCommitIterator(branch, iteratorOptions) {
        if (branch !== "main") throw new Error("Release proposal history must target main");
        let found = false;
        for await (const commit of options.github.mergeCommitIterator(branch, iteratorOptions)) {
          if (commit.sha === sha) found = true;
          if (found) yield commit;
        }
        if (!found) throw new Error("Fixed release base is absent from fetched main history");
      },
    };
    scoped = new Proxy(options.github, {
      get(target, property, receiver) {
        const override = Reflect.get(overrides, property);
        if (override !== undefined) return override;
        const value: unknown = Reflect.get(target, property, receiver);
        return typeof value === "function" ? value.bind(target) : value;
      },
    });
    contexts.set(scoped, { baseline, temporary, offline: options.offline ?? false });
    const manifest = await Manifest.fromManifest(
      scoped,
      "main",
      "release-please-config.json",
      ".release-please-manifest.json",
      { plugins: [PLUGIN] },
    );
    return await operation(manifest);
  } finally {
    if (scoped) contexts.delete(scoped);
    rmSync(temporary, { recursive: true, force: true });
  }
}
