import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { MEMBERS } from "../workspace-policy.ts";
import { PENDING, RELEASE_BRANCH } from "./control.ts";
import { ApiError } from "./github.ts";
import type { Request } from "./github.ts";
import { refreshReleaseLock } from "./refresh-lock.ts";
import { VERSION_FILES } from "./version.ts";

const directories: string[] = [];
afterEach(() => {
  for (const directory of directories.splice(0))
    rmSync(directory, { recursive: true, force: true });
});

function fixture(extraPath?: string, initial = false, proposedVersion?: string) {
  const root = mkdtempSync(join(tmpdir(), "lens-refresh-lock-test-"));
  directories.push(root);
  const git = (args: string[], input?: string, extraEnv: Record<string, string> = {}) =>
    execFileSync("git", ["-c", "core.hooksPath=/dev/null", ...args], {
      cwd: root,
      encoding: "utf8",
      input,
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: "Fixture",
        GIT_AUTHOR_EMAIL: "fixture@example.test",
        GIT_COMMITTER_NAME: "Fixture",
        GIT_COMMITTER_EMAIL: "fixture@example.test",
        ...extraEnv,
      },
      stdio: ["pipe", "pipe", "pipe"],
    }).trimEnd();
  git(["init", "-b", "main"]);
  for (const path of VERSION_FILES) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), readFileSync(path));
  }
  {
    const baselineVersion = initial ? "0.1.0" : "0.2.0";
    const current = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
    for (const path of VERSION_FILES) {
      let content = readFileSync(join(root, path), "utf8");
      if (path === "Cargo.lock") {
        for (const member of MEMBERS.filter((member) => member.ecosystem === "cargo"))
          content = content.replace(
            `name = "${member.name}"\nversion = "${current}"`,
            `name = "${member.name}"\nversion = "${baselineVersion}"`,
          );
      } else if (path === ".release-please-manifest.json")
        content = JSON.stringify(initial ? {} : { ".": baselineVersion });
      else
        content = content
          .replace(`"version": "${current}"`, `"version": "${baselineVersion}"`)
          .replace(`version = "${current}"`, `version = "${baselineVersion}"`);
      writeFileSync(join(root, path), content);
    }
  }
  git(["add", "."]);
  git(["commit", "-m", "feat: initial source"]);
  const source = git(["rev-parse", "HEAD"]);
  git(["update-ref", "refs/remotes/origin/main", source]);
  const current = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
  const version = proposedVersion ?? (initial ? "0.1.0" : "1.0.0");
  const baselineLock = readFileSync(join(root, "Cargo.lock"), "utf8");
  let refreshedLock = baselineLock;
  for (const member of MEMBERS.filter((member) => member.ecosystem === "cargo"))
    refreshedLock = refreshedLock.replace(
      `name = "${member.name}"\nversion = "${current}"`,
      `name = "${member.name}"\nversion = "${version}"`,
    );
  for (const path of VERSION_FILES.filter(
    (path) => path.endsWith(".json") || path === "Cargo.toml",
  )) {
    const content = readFileSync(join(root, path), "utf8");
    writeFileSync(
      join(root, path),
      path === ".release-please-manifest.json"
        ? JSON.stringify({ ".": version })
        : content
            .replace(`"version": "${current}"`, `"version": "${version}"`)
            .replace(`version = "${current}"`, `version = "${version}"`),
    );
  }
  writeFileSync(join(root, "CHANGELOG.md"), `## ${version}\n\nRelease fixture.\n`);
  if (extraPath) writeFileSync(join(root, extraPath), "untrusted candidate code");
  git(["add", "."]);
  git(["commit", "-m", `chore: release ${version}`]);
  const head = git(["rev-parse", "HEAD"]);
  git(["update-ref", "refs/pull/1/head", head]);
  git(["update-ref", `refs/heads/${RELEASE_BRANCH}`, head]);
  git(["reset", "--hard", source]);
  git(["remote", "add", "origin", root]);
  const pr = {
    number: 1,
    state: "open",
    merged: false,
    labels: [{ name: PENDING }],
    head: { ref: RELEASE_BRANCH, sha: head, repo: { full_name: "owner/repo" } },
    base: { ref: "main", repo: { full_name: "owner/repo" } },
  };
  const state = { count: 1, recheck: false, refConflict: false, reads: 0 };
  const writes: { path: string; method: string; body: unknown }[] = [];
  const request: Request = (async (path: string, method = "GET", body?: unknown) => {
    if (method !== "GET") writes.push({ path, method, body });
    if (path.startsWith("/pulls?")) {
      const query = new URL(path, "https://api.github.test").searchParams;
      assert.equal(query.get("state"), "open");
      assert.equal(query.get("base"), "main");
      assert.equal(query.get("head"), `owner:${RELEASE_BRANCH}`);
      return Array.from({ length: state.count }, (_, index) => ({ number: index + 1 }));
    }
    if (path === "/pulls/1") {
      state.reads++;
      if (state.recheck && state.reads === 2) return { ...pr, head: { ...pr.head, sha: source } };
      return structuredClone(pr);
    }
    if (path === "/git/blobs")
      return { sha: git(["hash-object", "-w", "--stdin"], (body as { content: string }).content) };
    if (path === "/git/trees") {
      const payload = body as {
        base_tree: string;
        tree: { sha: string; path: string; mode: string }[];
      };
      assert.equal(payload.base_tree, git(["rev-parse", `${pr.head.sha}^{tree}`]));
      assert.equal(payload.tree.length, 1);
      const index = join(root, ".git", "refresh-test-index");
      const env = { GIT_INDEX_FILE: index };
      git(["read-tree", payload.base_tree], undefined, env);
      const entry = payload.tree[0]!;
      assert.equal(entry.path, "Cargo.lock");
      git(
        ["update-index", "--add", "--cacheinfo", entry.mode, entry.sha, entry.path],
        undefined,
        env,
      );
      const sha = git(["write-tree"], undefined, env);
      rmSync(index);
      return { sha };
    }
    if (path === "/git/commits") {
      const payload = body as { tree: string; parents: string[]; message: string };
      assert.deepEqual(payload.parents, [pr.head.sha]);
      return {
        sha: git(["commit-tree", payload.tree, "-p", payload.parents[0]!, "-m", payload.message]),
      };
    }
    if (path === `/git/refs/heads/${RELEASE_BRANCH}`) {
      const payload = body as { sha: string; force: boolean };
      assert.equal(method, "PATCH");
      assert.equal(payload.force, false);
      if (state.refConflict) throw new ApiError(422, method, path);
      git(["merge-base", "--is-ancestor", pr.head.sha, payload.sha]);
      git(["update-ref", `refs/heads/${RELEASE_BRANCH}`, payload.sha, pr.head.sha]);
      git(["update-ref", "refs/pull/1/head", payload.sha]);
      pr.head.sha = payload.sha;
      return {};
    }
    throw new Error(`Unexpected API operation ${method} ${path}`);
  }) as Request;
  return { root, source, head, git, request, pr, state, writes, baselineLock, refreshedLock };
}

describe("owned release PR lock refresh", () => {
  it("returns absent without writes when there is no release proposal", async () => {
    const f = fixture();
    f.state.count = 0;
    await expect(
      refreshReleaseLock(f.request, f.root, f.source, "owner/repo", () => {
        throw new Error("unexpected refresh");
      }),
    ).resolves.toBe("absent");
    expect(f.writes).toEqual([]);
  });

  it("updates only the lock on the exact proposal head and is idempotent", async () => {
    const f = fixture();
    const refresh = (baseline: string, candidate: string) => {
      expect(baseline).toBe(f.root);
      expect(candidate).not.toBe(f.root);
      return f.refreshedLock;
    };
    await expect(
      refreshReleaseLock(f.request, f.root, f.source, "owner/repo", refresh),
    ).resolves.toBe("updated");
    expect(f.git(["diff", "--name-only", f.head, f.pr.head.sha])).toBe("Cargo.lock");
    expect(readFileSync(join(f.root, "Cargo.lock"), "utf8")).toBe(f.baselineLock);
    expect(f.git(["worktree", "list", "--porcelain"]).match(/^worktree /gmu)).toHaveLength(1);
    const writes = f.writes.length;
    await expect(
      refreshReleaseLock(f.request, f.root, f.source, "owner/repo", refresh),
    ).resolves.toBe("unchanged");
    expect(f.writes).toHaveLength(writes);
  });

  it("admits the initial 0.1.0 bootstrap without an unnecessary lock commit", async () => {
    const f = fixture(undefined, true);
    await expect(
      refreshReleaseLock(f.request, f.root, f.source, "owner/repo", () => f.baselineLock),
    ).resolves.toBe("unchanged");
    expect(f.writes).toEqual([]);
  });

  it.each(["0.2.0", "0.1.0"])(
    "rejects a non-advancing bootstrapped release: %s",
    async (version) => {
      const f = fixture(undefined, false, version);
      await expect(
        refreshReleaseLock(f.request, f.root, f.source, "owner/repo", () => f.refreshedLock),
      ).rejects.toThrow("must advance");
      expect(f.writes).toEqual([]);
    },
  );

  it("rejects an executable mode change even on an allowed manifest", async () => {
    const f = fixture();
    f.git(["checkout", RELEASE_BRANCH]);
    f.git(["update-index", "--chmod=+x", "package.json"]);
    f.git(["commit", "-m", "chore: alter manifest mode"]);
    f.pr.head.sha = f.git(["rev-parse", "HEAD"]);
    f.git(["update-ref", "refs/pull/1/head", f.pr.head.sha]);
    f.git(["checkout", "--force", "main"]);
    await expect(
      refreshReleaseLock(f.request, f.root, f.source, "owner/repo", () => f.refreshedLock),
    ).rejects.toThrow("only existing version files");
    expect(f.writes).toEqual([]);
  });

  it("rejects a candidate that does not contain the trusted source", async () => {
    const f = fixture();
    f.git(["commit", "--allow-empty", "-m", "feat: newer source"]);
    const source = f.git(["rev-parse", "HEAD"]);
    f.git(["update-ref", "refs/remotes/origin/main", source]);
    await expect(
      refreshReleaseLock(f.request, f.root, source, "owner/repo", () => f.refreshedLock),
    ).rejects.toThrow(/.+/u);
    expect(f.writes).toEqual([]);
  });

  it.each(["multiple", "closed", "merged", "label", "fork", "branch", "base", "fetched-sha"])(
    "rejects a conflicting proposal identity: %s",
    async (kind) => {
      const f = fixture();
      if (kind === "multiple") f.state.count = 2;
      if (kind === "closed") f.pr.state = "closed";
      if (kind === "merged") f.pr.merged = true;
      if (kind === "label") f.pr.labels = [];
      if (kind === "fork") f.pr.head.repo.full_name = "other/repo";
      if (kind === "branch") f.pr.head.ref = "other";
      if (kind === "base") f.pr.base.ref = "other";
      if (kind === "fetched-sha") f.pr.head.sha = f.source;
      await expect(
        refreshReleaseLock(f.request, f.root, f.source, "owner/repo", () => f.refreshedLock),
      ).rejects.toThrow(/.+/u);
      expect(f.writes).toEqual([]);
    },
  );

  it("rejects candidate code changes before invoking Cargo", async () => {
    const f = fixture("untrusted.ts");
    let called = false;
    await expect(
      refreshReleaseLock(f.request, f.root, f.source, "owner/repo", () => {
        called = true;
        return f.refreshedLock;
      }),
    ).rejects.toThrow("only existing version files");
    expect(called).toBe(false);
    expect(f.writes).toEqual([]);
  });

  it.each(["unsynchronized", "other-file", "helper-failure"])(
    "cleans up and rejects %s output",
    async (kind) => {
      const f = fixture();
      await expect(
        refreshReleaseLock(f.request, f.root, f.source, "owner/repo", (_baseline, candidate) => {
          if (kind === "helper-failure") throw new Error("dependency drift");
          if (kind === "other-file") writeFileSync(join(candidate, "untrusted.ts"), "changed");
          return kind === "unsynchronized" ? f.baselineLock : f.refreshedLock;
        }),
      ).rejects.toThrow(/.+/u);
      expect(f.writes).toEqual([]);
      expect(f.git(["worktree", "list", "--porcelain"]).match(/^worktree /gmu)).toHaveLength(1);
    },
  );

  it.each(["recheck", "refConflict"] as const)(
    "does not overwrite concurrent changes: %s",
    async (kind) => {
      const f = fixture();
      f.state[kind] = true;
      await expect(
        refreshReleaseLock(f.request, f.root, f.source, "owner/repo", () => f.refreshedLock),
      ).rejects.toThrow(/.+/u);
      expect(f.pr.head.sha).toBe(f.head);
      expect(f.writes.filter((write) => write.method === "PATCH")).toHaveLength(
        kind === "recheck" ? 0 : 1,
      );
    },
  );
});
