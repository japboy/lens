import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { control, PENDING, RELEASE_BRANCH, TAGGED } from "./control.ts";
import type { PullRequest } from "./control.ts";
import { ApiError } from "./github.ts";
import type { Request } from "./github.ts";
import { VERSION_FILES } from "./version.ts";
import { annotation } from "./source.ts";

function fixture(unrelatedCount = 0) {
  const root = mkdtempSync(join(tmpdir(), "lens-release-control-"));
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "core.hooksPath=/dev/null", ...args], {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: "Fixture",
        GIT_AUTHOR_EMAIL: "fixture@example.test",
        GIT_COMMITTER_NAME: "Fixture",
        GIT_COMMITTER_EMAIL: "fixture@example.test",
      },
    }).trim();
  git("init", "-b", "main");
  for (const path of VERSION_FILES) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), readFileSync(path));
  }
  writeFileSync(join(root, ".release-please-manifest.json"), "{}\n");
  git("add", ".");
  git("commit", "-m", "feat: initial source");
  const initial = git("rev-parse", "HEAD");
  const advance = () => git("update-ref", "refs/remotes/origin/main", git("rev-parse", "HEAD"));
  advance();
  let merged: PullRequest | undefined;
  let tags: { name: string }[] = [];
  let releases: { tag_name: string; draft: boolean; prerelease: boolean }[] = [];
  let ref: { object: { type: string; sha: string } } | undefined;
  let tag: unknown;
  const writes: string[] = [];
  const reads: string[] = [];
  const request = (async (path: string, method = "GET", payload?: unknown) => {
    if (method !== "GET") writes.push(method + " " + path);
    if (path.startsWith("/commits/")) return [];
    reads.push(path);
    if (path.startsWith("/issues?")) throw new ApiError(403, method, path);
    if (path.startsWith("/pulls?")) {
      const query = new URL(path, "https://api.github.test").searchParams;
      if (
        query.get("state") !== "closed" ||
        query.get("base") !== "main" ||
        query.get("head") !== `owner:${RELEASE_BRANCH}`
      )
        throw new Error("Unexpected release PR search scope");
      const entries = [
        ...Array.from({ length: unrelatedCount }, (_, index) => ({
          number: index + 2,
          labels: [{ name: TAGGED }],
        })),
        ...(merged ? [merged] : []),
      ];
      const start = (Number(query.get("page")) - 1) * 100;
      return entries.slice(start, start + 100);
    }
    if (path === "/pulls/1") return merged;
    if (path.startsWith("/tags?")) return tags;
    if (path.startsWith("/releases?")) return releases;
    if (path.startsWith("/releases/tags/")) {
      const release = releases.find((item) => item.tag_name === path.split("/").at(-1));
      if (!release) throw new ApiError(404, method, path);
      return release;
    }
    if (path === "/git/ref/tags/v0.1.0") {
      if (!ref) throw new ApiError(404, method, path);
      return ref;
    }
    if (path === "/git/tags" && method === "POST") {
      const data = payload as { tag: string; message: string; object: string };
      tag = { tag: data.tag, message: data.message, object: { type: "commit", sha: data.object } };
      return { sha: "a".repeat(40) };
    }
    if (path === "/git/refs" && method === "POST") {
      ref = { object: { type: "tag", sha: "a".repeat(40) } };
      tags = [{ name: "v0.1.0" }];
      return ref;
    }
    if (path === "/git/tags/" + "a".repeat(40)) return tag;
    if (path === "/issues/1/labels" && method === "POST") {
      merged!.labels.push({ name: TAGGED });
      return [];
    }
    if (path === "/issues/1/labels/autorelease%3A%20pending" && method === "DELETE") {
      merged!.labels = merged!.labels.filter((label) => label.name !== PENDING);
      return [];
    }
    throw new Error("Unexpected request: " + method + " " + path);
  }) as Request;
  return {
    root,
    git,
    initial,
    advance,
    request,
    writes,
    reads,
    tag: () => tag,
    merge: (sha: string) => {
      merged = {
        number: 1,
        state: "closed",
        merged: true,
        merge_commit_sha: sha,
        base: { ref: "main", repo: { full_name: "owner/repo" } },
        head: { ref: RELEASE_BRANCH, repo: { full_name: "owner/repo" } },
        labels: [{ name: PENDING }],
      };
    },
    setTags: () => {
      tags = [{ name: "v0.1.0" }];
    },
    setRelease: (draft: boolean) => {
      releases = [{ tag_name: "v0.1.0", draft, prerelease: false }];
    },
    cleanup: () => rmSync(root, { recursive: true, force: true }),
  };
}

describe("standard initial release control", () => {
  it("delegates empty-manifest runs to Release Please without creating PRs or files", async () => {
    const f = fixture();
    try {
      for (let attempt = 0; attempt < 2; attempt++)
        await expect(control(f.request, f.root, f.initial, "owner/repo")).resolves.toBe(
          "update-pr",
        );
      expect(f.writes).toEqual([]);
      expect(f.git("status", "--porcelain")).toBe("");
    } finally {
      f.cleanup();
    }
  });
  it.each(["tag", "release"])("rejects existing %s state with an empty manifest", async (kind) => {
    const f = fixture();
    try {
      if (kind === "tag") f.setTags();
      else f.setRelease(false);
      await expect(control(f.request, f.root, f.initial, "owner/repo")).rejects.toThrow(
        "empty manifest",
      );
      expect(f.writes).toEqual([]);
    } finally {
      f.cleanup();
    }
  });
  it.each([0, 100])(
    "recovers a merged PR behind %i unrelated closed PRs after main advances",
    async (unrelatedCount) => {
      const f = fixture(unrelatedCount);
      try {
        writeFileSync(join(f.root, ".release-please-manifest.json"), '{".":"0.1.0"}\n');
        writeFileSync(
          join(f.root, "CHANGELOG.md"),
          "# Changelog\n\n## 0.1.0\n\n### Features\n\n- Initial capability.\n",
        );
        f.git("add", ".");
        f.git("commit", "-m", "chore(main): release 0.1.0");
        const merge = f.git("rev-parse", "HEAD");
        f.merge(merge);
        f.git("commit", "--allow-empty", "-m", "fix: later change");
        f.advance();
        const tip = f.git("rev-parse", "HEAD");
        await expect(control(f.request, f.root, tip, "owner/repo")).resolves.toBe("tagged");
        expect(f.tag()).toEqual({
          tag: "v0.1.0",
          message: annotation({ schema: 1, version: "0.1.0", commit: merge, pullRequest: 1 }),
          object: { type: "commit", sha: merge },
        });
        const writes = [...f.writes];
        await expect(control(f.request, f.root, tip, "owner/repo")).resolves.toBe(
          "awaiting-publication",
        );
        f.setRelease(true);
        await expect(control(f.request, f.root, tip, "owner/repo")).resolves.toBe(
          "awaiting-publication",
        );
        f.setRelease(false);
        await expect(control(f.request, f.root, tip, "owner/repo")).resolves.toBe("update-pr");
        expect(f.writes).toEqual(writes);
        expect(f.reads.some((path) => path.startsWith("/issues?"))).toBe(false);
        expect(
          f.reads
            .filter((path) => /^\/pulls\/\d+$/u.test(path))
            .every((path) => path === "/pulls/1"),
        ).toBe(true);
        expect(f.reads.some((path) => path.startsWith("/pulls?") && path.includes("page=2"))).toBe(
          unrelatedCount > 0,
        );
      } finally {
        f.cleanup();
      }
    },
  );
});
