import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { VERSION } from "release-please";
import { assertReleasePleaseVersion } from "./library-version.ts";
import type { Scm } from "release-please/build/src/scm.js";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { withReleaseProposalManifest } from "./proposal.ts";
import { snapshotReleaseSource, verifyReleaseDelta } from "./release-delta.ts";
import { installationNotes, releaseNotes } from "./artifact.ts";

const root = fileURLToPath(new URL("../../", import.meta.url));
const baseSha = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const previous = JSON.parse(readFileSync(new URL("../../package.json", import.meta.url), "utf8"))
  .version as string;
const oldSha = "a".repeat(40);
afterEach(() => vi.useRealTimers());

// Populate a cold registry once from the exact trusted input. Every assertion
// below still runs Cargo offline, including checksum-failure and delta gates.
beforeAll(() => {
  const temporary = mkdtempSync(join(tmpdir(), "lens-proposal-dependencies-"));
  try {
    const snapshot = snapshotReleaseSource(root, baseSha, join(temporary, "baseline"));
    execFileSync("cargo", ["fetch", "--locked"], {
      cwd: snapshot.directory,
      timeout: 120_000,
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}, 130_000);

function github(options: { basePresent?: boolean; maintenance?: boolean; sha?: string } = {}): {
  scm: Scm;
  writes: string[];
} {
  const writes: string[] = [];
  const scm = {
    repository: { owner: "fixture", repo: "lens", defaultBranch: "main" },
    async *releaseIterator() {
      yield { id: 1, tagName: `v${previous}`, sha: oldSha, url: "https://example.invalid/release" };
    },
    async *tagIterator() {
      yield { name: `v${previous}`, sha: oldSha };
    },
    async *mergeCommitIterator() {
      yield {
        sha: "b".repeat(40),
        message: "feat: after fixed base must be excluded",
        files: ["package.json"],
      };
      if (options.basePresent !== false)
        yield {
          sha: options.sha ?? baseSha,
          message: options.maintenance ? "docs: maintenance" : "fix: release proof",
          files: ["package.json"],
        };
      yield { sha: oldSha, message: `chore: release ${previous}`, files: ["package.json"] };
    },
    async *pullRequestIterator() {},
    async createPullRequest() {
      writes.push("create");
      throw new Error("Test does not permit remote writes");
    },
  } as unknown as Scm;
  return { scm, writes };
}

describe("pinned Release Please Cargo proposal", () => {
  it("generates complete manifest+Cargo updates from the fixed snapshot with the adopted library", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-09-18T00:00:00Z"));
    assertReleasePleaseVersion(
      JSON.parse(readFileSync(join(root, "package.json"), "utf8")).devDependencies[
        "release-please"
      ],
      JSON.parse(readFileSync(join(root, "release-please-config.json"), "utf8")).$schema,
      VERSION,
    );
    const { scm, writes } = github();
    const proposals = await withReleaseProposalManifest(
      { github: scm, root, baseSha, offline: true },
      (manifest) => manifest.buildPullRequests(),
    );
    expect(proposals).toHaveLength(1);
    const updates = proposals[0]!.updates;
    expect(new Set(updates.map((update) => update.path)).size).toBe(updates.length);
    expect(updates.map((update) => update.path)).toEqual(
      expect.arrayContaining([
        "Cargo.lock",
        "Cargo.toml",
        "package.json",
        ".release-please-manifest.json",
        "CHANGELOG.md",
      ]),
    );
    const version = previous.replace(/\d+$/u, (patch) => String(Number(patch) + 1));
    expect(
      JSON.parse(
        updates.find((update) => update.path === "package.json")!.updater.updateContent("ignored"),
      ).version,
    ).toBe(version);
    expect(proposals[0]!.body.toString()).not.toContain("after fixed base");
    expect(proposals[0]!.body.toString().split("## Install and update")).toHaveLength(2);
    expect(
      updates.find((update) => update.path === "CHANGELOG.md")!.updater.updateContent(undefined),
    ).not.toContain("## Install and update");
    const second = await withReleaseProposalManifest(
      { github: scm, root, baseSha, offline: true },
      (manifest) => manifest.buildPullRequests(),
    );
    const render = (proposal: (typeof proposals)[number]) => ({
      title: proposal.title.toString(),
      body: proposal.body.toString(),
      files: proposal.updates.map((update) => [
        update.path,
        update.updater.updateContent(undefined),
      ]),
    });
    expect(second.map(render)).toEqual(proposals.map(render));
    expect(writes).toEqual([]);
  }, 120_000);

  it("does not invoke a writer for non-release commits", async () => {
    const { scm, writes } = github({ maintenance: true });
    const result = await withReleaseProposalManifest(
      { github: scm, root, baseSha, offline: true },
      (manifest) => manifest.createPullRequests(),
    );
    expect(result).toEqual([]);
    expect(writes).toEqual([]);
  }, 120_000);

  it("fails closed when history does not contain the fixed base", async () => {
    const { scm, writes } = github({ basePresent: false });
    await expect(
      withReleaseProposalManifest({ github: scm, root, baseSha, offline: true }, (manifest) =>
        manifest.createPullRequests(),
      ),
    ).rejects.toThrow("Fixed release base");
    expect(writes).toEqual([]);
  });

  it("rejects real Cargo checksum failure before invoking any remote writer", async () => {
    const temporary = mkdtempSync(join(tmpdir(), "lens-proposal-failure-"));
    try {
      const fixtureRoot = join(temporary, "repository");
      snapshotReleaseSource(root, baseSha, fixtureRoot);
      const lockPath = join(fixtureRoot, "Cargo.lock");
      writeFileSync(
        lockPath,
        readFileSync(lockPath, "utf8").replace(
          /^checksum = "[^"]+"$/mu,
          `checksum = "${"0".repeat(64)}"`,
        ),
      );
      const git = (...args: string[]) =>
        execFileSync("git", args, {
          cwd: fixtureRoot,
          encoding: "utf8",
          stdio: ["ignore", "pipe", "pipe"],
        }).trim();
      git("init", "-q");
      git("add", ".");
      git(
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "-qm",
        "fixture",
      );
      const sha = git("rev-parse", "HEAD");
      const { scm, writes } = github({ sha });
      await expect(
        withReleaseProposalManifest(
          { github: scm, root: fixtureRoot, baseSha: sha, offline: true },
          (manifest) => manifest.createPullRequests(),
        ),
      ).rejects.toThrow(/checksum/iu);
      expect(writes).toEqual([]);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  }, 120_000);

  it("preserves installation guidance through standard single-component release extraction", async () => {
    const temporary = mkdtempSync(join(tmpdir(), "lens-proposal-notes-"));
    try {
      const fixtureRoot = join(temporary, "repository");
      snapshotReleaseSource(root, baseSha, fixtureRoot);
      const configPath = join(fixtureRoot, "release-please-config.json");
      const config = JSON.parse(readFileSync(configPath, "utf8")) as Record<string, unknown>;
      config["separate-pull-requests"] = true;
      writeFileSync(configPath, JSON.stringify(config));
      const git = (...args: string[]) =>
        execFileSync("git", args, {
          cwd: fixtureRoot,
          encoding: "utf8",
          stdio: ["ignore", "pipe", "pipe"],
        }).trim();
      git("init", "-q");
      git("add", ".");
      git(
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "-qm",
        "fixture",
      );
      const sha = git("rev-parse", "HEAD");
      const { scm, writes } = github({ sha });
      await withReleaseProposalManifest(
        { github: scm, root: fixtureRoot, baseSha: sha, offline: true },
        async (manifest) => {
          const proposals = await manifest.buildPullRequests();
          const proposal = proposals[0]!;
          scm.pullRequestIterator = async function* (_branch, status) {
            if (status === "MERGED")
              yield {
                number: 94,
                headBranchName: proposal.headRefName,
                baseBranchName: "main",
                title: proposal.title.toString(),
                body: proposal.body.toString(),
                labels: ["autorelease: pending"],
                files: proposal.updates.map((update) => update.path),
                sha,
              };
          };
          const releases = await manifest.buildReleases();
          expect(releases).toHaveLength(1);
          const version = proposal.version!.toString();
          expect(releases[0]!.notes).toContain(installationNotes(version));
          expect(releases[0]!.notes!.split("## Install and update")).toHaveLength(2);
          for (const update of proposal.updates)
            writeFileSync(join(fixtureRoot, update.path), update.updater.updateContent(undefined));
          const commit = (message: string) => {
            git("add", ".");
            git(
              "-c",
              "core.hooksPath=/dev/null",
              "-c",
              "user.name=Fixture",
              "-c",
              "user.email=test@example.invalid",
              "commit",
              "-qm",
              message,
            );
            return git("rev-parse", "HEAD");
          };
          const head = commit("release proposal");
          expect(verifyReleaseDelta(fixtureRoot, sha, head, true).version).toBe(version);
          const packagePath = join(fixtureRoot, "package.json");
          const packageData = JSON.parse(readFileSync(packagePath, "utf8")) as Record<
            string,
            unknown
          >;
          packageData.name = "unexpected-package-change";
          writeFileSync(packagePath, JSON.stringify(packageData));
          const invalid = commit("non-version change");
          expect(() => verifyReleaseDelta(fixtureRoot, sha, invalid, true)).toThrow(
            "non-version manifest data",
          );
        },
      );
      expect(writes).toEqual([]);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  }, 120_000);

  it("keeps legacy guidance bytes shared with the new proposal guidance", () => {
    const version = "0.1.0";
    const legacy = releaseNotes(
      "section",
      { tag: "v0.1.0", source: oldSha, repository: "fixture/lens", runId: "1", previousTag: null },
      { name: "Lens_0.1.0_aarch64.dmg", sha256: "0".repeat(64), size: 1 },
    );
    expect(
      legacy.slice(legacy.indexOf("## Install and update"), legacy.indexOf("\n\n## Provenance")),
    ).toBe(installationNotes(version));
  });
});
