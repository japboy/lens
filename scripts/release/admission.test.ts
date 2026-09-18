import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { MEMBERS } from "../workspace-policy.ts";
import { admitRelease } from "./admission.ts";
import type { PullRequest, Release } from "./control.ts";
import type { Request } from "./github.ts";
import { annotation } from "./source.ts";

function fixture(legacy: boolean) {
  const root = mkdtempSync(join(tmpdir(), "lens-tag-admission-"));
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
  const files: Record<string, string> = {
    "apps/desktop/src-tauri/tauri.conf.json": '{"version":"0.1.0"}',
    "Cargo.toml": '[workspace.package]\nversion = "0.1.0"\n',
    ".release-please-manifest.json": '{".":"0.1.0"}',
    "CHANGELOG.md": "# Changelog\n\n## 0.1.0\n\nInitial release.\n",
    "Cargo.lock": "",
  };
  for (const member of MEMBERS)
    if (member.ecosystem === "cargo") {
      files[`${member.directory}/Cargo.toml`] =
        `[package]\nname = "${member.name}"\nversion.workspace = true\n`;
      files["Cargo.lock"] += `[[package]]\nname = "${member.name}"\nversion = "0.1.0"\n\n`;
    } else if (member.ecosystem === "pnpm")
      files[join(member.directory, "package.json")] = '{"version":"0.1.0"}';
  git("init", "-b", "main");
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  git("add", ".");
  git("commit", "-m", "chore(main): release 0.1.0");
  const source = git("rev-parse", "HEAD");
  git("update-ref", "refs/remotes/origin/main", source);
  if (legacy)
    git(
      "tag",
      "-a",
      "v0.1.0",
      "-m",
      annotation({ schema: 1, version: "0.1.0", commit: source, pullRequest: 18 }),
    );
  else git("tag", "v0.1.0");
  const pr: PullRequest = {
    number: 18,
    merged: true,
    state: "closed",
    merge_commit_sha: source,
    base: { ref: "main", repo: { full_name: "owner/repo" } },
    head: {
      ref: legacy
        ? "release-please--branches--main"
        : "release-please--branches--main--components--lens",
      repo: { full_name: "owner/repo" },
    },
    labels: [],
  };
  const release: Release = {
    id: 50,
    tag_name: "v0.1.0",
    target_commitish: source,
    draft: true,
    prerelease: false,
    name: "standard title",
    body: "standard body",
    upload_url: "unused",
  };
  const remote = {
    object: { type: legacy ? "tag" : "commit", sha: legacy ? git("rev-parse", "v0.1.0") : source },
  };
  const flags = { duplicate: false, wrongAnnotationSource: false };
  const request = (async (path: string, method = "GET") => {
    expect(method).toBe("GET");
    if (path.startsWith("/commits/"))
      return flags.duplicate ? [{ number: 18 }, { number: 19 }] : [{ number: 18 }];
    if (path.startsWith("/pulls/")) return { ...pr, number: Number(path.split("/").at(-1)) };
    if (path.startsWith("/git/ref/")) return remote;
    if (path.startsWith("/git/tags/"))
      return {
        object: { type: "commit", sha: flags.wrongAnnotationSource ? "c".repeat(40) : source },
      };
    if (path.startsWith("/releases?")) return [release];
    throw new Error(`Unexpected read ${path}`);
  }) as Request;
  return {
    root,
    source,
    pr,
    release,
    remote,
    flags,
    request,
    close: () => rmSync(root, { recursive: true, force: true }),
  };
}

describe("read-only dual-format release admission", () => {
  it.each([false, true])(
    "resolves exact merged/version source for legacy=%s without label dependence",
    async (legacy) => {
      const f = fixture(legacy);
      try {
        expect(await admitRelease(f.request, f.root, "owner/repo", "v0.1.0")).toEqual({
          tag: "v0.1.0",
          version: "0.1.0",
          source: f.source,
          pullRequest: 18,
          releaseId: 50,
          legacy,
          draft: true,
        });
      } finally {
        f.close();
      }
    },
  );
  it.each(["tag", "target", "pr", "duplicate", "fork"])(
    "rejects %s identity conflicts",
    async (fault) => {
      const f = fixture(false);
      try {
        if (fault === "tag") f.remote.object.sha = "b".repeat(40);
        if (fault === "target") f.release.target_commitish = "main";
        if (fault === "pr") f.pr.merge_commit_sha = "b".repeat(40);
        if (fault === "duplicate") f.flags.duplicate = true;
        if (fault === "fork") f.pr.head.repo!.full_name = "fork/repo";
        await expect(admitRelease(f.request, f.root, "owner/repo", "v0.1.0")).rejects.toThrow(
          /.+/u,
        );
      } finally {
        f.close();
      }
    },
  );
  it("rejects remote annotated target divergence", async () => {
    const f = fixture(true);
    try {
      f.flags.wrongAnnotationSource = true;
      await expect(admitRelease(f.request, f.root, "owner/repo", "v0.1.0")).rejects.toThrow(
        "tag/source conflict",
      );
    } finally {
      f.close();
    }
  });
});
