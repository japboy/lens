import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { VERSION_FILES } from "./version.ts";
import { describe, expect, it } from "vitest";
import { ApiError } from "./github.ts";
import type { GitHub, Request } from "./github.ts";
import {
  ensureTag,
  previousRelease,
  validateReleasePr,
  RELEASE_BRANCH,
  PENDING,
} from "./control.ts";
import type { PullRequest, Release } from "./control.ts";
import { annotation, assertPrTitle, changelogSection } from "./source.ts";
import { sha256, verifyArtifact } from "./artifact.ts";
import type { ReleaseManifest } from "./artifact.ts";
import { publish, requireReleaseJobs } from "./publish.ts";

const source = "a".repeat(40);
const identity = { tag: "v0.1.0", source, repository: "owner/repo", runId: "123" };
const pr = (): PullRequest => ({
  number: 18,
  merged: true,
  state: "closed",
  merge_commit_sha: source,
  base: { ref: "main", repo: { full_name: "owner/repo" } },
  head: { ref: RELEASE_BRANCH, repo: { full_name: "owner/repo" } },
  labels: [{ name: PENDING }],
});

describe("merged release authority", () => {
  it("requires repository, branch, merge commit and release labels jointly", () => {
    expect(() => validateReleasePr(pr(), source, "owner/repo")).not.toThrow(/.+/u);
    const cases = [
      { ...pr(), merged: false },
      { ...pr(), merge_commit_sha: "b".repeat(40) },
      { ...pr(), labels: [] },
      { ...pr(), head: { ...pr().head, ref: "feature" } },
      { ...pr(), head: { ...pr().head, repo: { full_name: "fork/repo" } } },
      { ...pr(), base: { ...pr().base, ref: "other" } },
    ];
    for (const value of cases)
      expect(() => validateReleasePr(value, source, "owner/repo")).toThrow(/.+/u);
  });
  it("validates new squash titles and newest nonempty unique changelog sections", () => {
    for (const title of [
      "feat: support another source",
      "fix(ui): repair layout",
      "chore(main): release 0.1.0",
      "feat!: change contract",
    ])
      expect(() => assertPrTitle(title)).not.toThrow(/.+/u);
    for (const title of ["updated", "feat: ", "feat: line\nother", "anything: title"])
      expect(() => assertPrTitle(title)).toThrow(/.+/u);
    expect(
      changelogSection(
        "# Changelog\n\n## [0.2.0](link) (date)\n\n- New.\n\n## 0.1.0\nOld.",
        "0.2.0",
      ),
    ).toContain("New.");
    for (const content of [
      "## 0.1.0\n",
      "## 0.1.0\nNew\n## 0.1.0\nOld",
      "## 0.2.0\nNew\n## 0.1.0\nOld",
    ])
      expect(() => changelogSection(content, "0.1.0")).toThrow(/.+/u);
  });
  it("creates an annotated object once, reuses it, and rejects lightweight or conflicting tags", async () => {
    const data = { schema: 1 as const, version: "0.1.0", commit: source, pullRequest: 18 };
    let ref: { object: { type: string; sha: string } } | undefined;
    let tag = { tag: "v0.1.0", message: annotation(data), object: { type: "commit", sha: source } };
    const writes: string[] = [];
    const request = (async (path: string, method = "GET") => {
      if (method === "POST") writes.push(path);
      if (path.startsWith("/git/ref/")) {
        if (!ref) throw new ApiError(404, "GET", path);
        return ref;
      }
      if (path === "/git/tags") return { sha: "c".repeat(40) };
      if (path === "/git/refs") {
        ref = { object: { type: "tag", sha: "c".repeat(40) } };
        return ref;
      }
      return tag;
    }) as Request;
    await ensureTag(request, data);
    await ensureTag(request, data);
    expect(writes).toEqual(["/git/tags", "/git/refs"]);
    ref!.object.type = "commit";
    await expect(ensureTag(request, data)).rejects.toThrow("lightweight");
    ref!.object.type = "tag";
    tag = { ...tag, object: { type: "commit", sha: "b".repeat(40) } };
    await expect(ensureTag(request, data)).rejects.toThrow("conflicts");
  });
  it("blocks unpublished earlier tags, version rollback and noninitial first versions", async () => {
    let tags: { name: string }[] = [];
    let releases: Partial<Release>[] = [];
    const request = (async (path: string) =>
      path.startsWith("/tags") ? tags : releases) as Request;
    await expect(previousRelease(request, "0.1.0")).resolves.toBeUndefined();
    await expect(previousRelease(request, "0.2.0")).rejects.toThrow("First release");
    tags = [{ name: "v0.1.0" }];
    await expect(previousRelease(request, "0.2.0")).rejects.toThrow("not published");
    releases = [{ tag_name: "v0.1.0", draft: false, prerelease: false }];
    await expect(previousRelease(request, "0.2.0")).resolves.toBe("v0.1.0");
    tags.push({ name: "v0.3.0" });
    await expect(previousRelease(request, "0.2.0")).rejects.toThrow("monotonically");
  });
});

function fixture() {
  const directory = mkdtempSync(join(tmpdir(), "lens-release-state-"));
  const bytes = Buffer.from("verified DMG fixture");
  const name = "Lens_0.1.0_aarch64.dmg";
  const sums = Buffer.from(`${sha256(bytes)}  ${name}\n`);
  const body = "## 0.1.0\n\nInitial fixture.\n";
  const manifest: ReleaseManifest = {
    schema: 1,
    ...identity,
    version: "0.1.0",
    runAttempt: "1",
    previousTag: null,
    assets: [
      { name, size: bytes.length, sha256: sha256(bytes) },
      { name: "SHA256SUMS", size: sums.length, sha256: sha256(sums) },
    ],
    notesSha256: sha256(body),
    tools: {},
    configuration: {},
    applicationSignature: "adhoc",
    dmgSignature: "unsigned",
    notarization: "not-performed",
  };
  writeFileSync(join(directory, name), bytes);
  writeFileSync(join(directory, "SHA256SUMS"), sums);
  writeFileSync(join(directory, "release-notes.md"), body);
  writeFileSync(join(directory, "release-manifest.json"), JSON.stringify(manifest));
  let release: Release | undefined;
  let immutable = true;
  let failUpload = false;
  let losePublish = false;
  const remote: {
    id: number;
    name: string;
    size: number;
    state: string;
    digest: string | null;
    bytes: Buffer;
  }[] = [];
  const writes: string[] = [];
  const request = (async (path: string, method = "GET", payload?: unknown) => {
    if (method !== "GET") writes.push(method + " " + path);
    if (path.startsWith("/releases/tags/")) {
      if (!release) throw new ApiError(404, method, path);
      return release;
    }
    if (path === "/immutable-releases") return { enabled: immutable, enforced_by_owner: false };
    if (path.startsWith("/releases/1/assets?")) return remote;
    if (path === "/releases" && method === "POST") {
      release = {
        id: 1,
        upload_url: "https://uploads.github.com/repos/owner/repo/releases/1/assets{?name}",
        ...(payload as Omit<Release, "id" | "upload_url">),
      };
      return release;
    }
    if (path === "/releases/1" && method === "PATCH") {
      release!.draft = false;
      if (losePublish) {
        losePublish = false;
        throw new Error("lost publication response");
      }
      return release;
    }
    if (path.startsWith("/releases?")) return release ? [release] : [];
    throw new Error("Unexpected endpoint " + path);
  }) as Request;
  const api: GitHub = {
    request,
    upload: async (_url, assetName, content) => {
      writes.push("upload " + assetName);
      if (failUpload && assetName === "SHA256SUMS") {
        failUpload = false;
        throw new Error("partial upload");
      }
      remote.push({
        id: remote.length + 1,
        name: assetName,
        size: content.length,
        state: "uploaded",
        digest: "sha256:" + sha256(content),
        bytes: content,
      });
    },
    download: async (path) =>
      remote.find((asset) => String(asset.id) === path.split("/").at(-1))!.bytes,
  };
  return {
    directory,
    api,
    remote,
    writes,
    manifest,
    cleanup: () => rmSync(directory, { recursive: true, force: true }),
    release: () => release!,
    policy: () => immutable,
    immutable: (value: boolean) => {
      immutable = value;
    },
    failUpload: () => {
      failUpload = true;
    },
    losePublish: () => {
      losePublish = true;
    },
  };
}

describe("verified artifact and finite publisher", () => {
  it("accepts only a complete same-source original artifact", () => {
    const f = fixture();
    try {
      expect(verifyArtifact(f.directory, identity)).toEqual(f.manifest);
      for (const changed of [
        { ...identity, source: "b".repeat(40) },
        { ...identity, runId: "456" },
        { ...identity, tag: "v0.2.0" },
      ])
        expect(() => verifyArtifact(f.directory, changed)).toThrow(/.+/u);
      writeFileSync(join(f.directory, "extra"), "");
      expect(() => verifyArtifact(f.directory, identity)).toThrow("four regular");
    } finally {
      f.cleanup();
    }
  });
  it("holds a draft, resumes the exact two assets, then makes publication terminal", async () => {
    const f = fixture();
    try {
      await expect(publish(f.api, f.directory, identity, false, f.policy())).resolves.toBe("draft");
      expect(f.release().draft).toBe(true);
      await expect(publish(f.api, f.directory, identity, true, f.policy())).resolves.toBe(
        "published",
      );
      const writes = [...f.writes];
      await expect(publish(f.api, f.directory, identity, true, f.policy())).resolves.toBe(
        "already-published",
      );
      expect(f.writes).toEqual(writes);
      expect(f.remote.map((asset) => asset.name)).toEqual(
        f.manifest.assets.map((asset) => asset.name),
      );
    } finally {
      f.cleanup();
    }
  });
  it("recovers a partial upload without replacing the first asset", async () => {
    const f = fixture();
    try {
      f.failUpload();
      await expect(publish(f.api, f.directory, identity, true, f.policy())).rejects.toThrow(
        "partial upload",
      );
      expect(f.release().draft).toBe(true);
      await expect(publish(f.api, f.directory, identity, true, f.policy())).resolves.toBe(
        "published",
      );
      expect(f.writes.filter((value) => value === "upload Lens_0.1.0_aarch64.dmg")).toHaveLength(1);
    } finally {
      f.cleanup();
    }
  });
  it("recovers a lost publication response by verifying the terminal release", async () => {
    const f = fixture();
    try {
      f.losePublish();
      await expect(publish(f.api, f.directory, identity, true, f.policy())).rejects.toThrow(
        "lost publication",
      );
      await expect(publish(f.api, f.directory, identity, true, f.policy())).resolves.toBe(
        "already-published",
      );
      expect(f.writes.filter((value) => value.startsWith("PATCH"))).toHaveLength(1);
    } finally {
      f.cleanup();
    }
  });
  it.each(["digest", "bytes", "size", "extra", "metadata", "immutability"])(
    "fails closed on %s conflicts",
    async (kind) => {
      const f = fixture();
      try {
        await publish(f.api, f.directory, identity, false, f.policy());
        if (kind === "digest") f.remote[0]!.digest = "sha256:wrong";
        if (kind === "bytes") {
          f.remote[0]!.digest = null;
          f.remote[0]!.bytes = Buffer.from("different");
        }
        if (kind === "size") f.remote[0]!.size++;
        if (kind === "extra") f.remote.push({ ...f.remote[0]!, name: "extra", id: 3 });
        if (kind === "metadata") f.release().target_commitish = "b".repeat(40);
        if (kind === "immutability") f.immutable(false);
        await expect(publish(f.api, f.directory, identity, true, f.policy())).rejects.toThrow(
          /.+/u,
        );
        expect(f.release().draft).toBe(true);
        expect(f.writes.some((value) => value.startsWith("DELETE"))).toBe(false);
      } finally {
        f.cleanup();
      }
    },
  );
  it("requires every verification job even when original assets are reused", () => {
    for (const portable of ["success", "failure", "cancelled", "skipped", ""])
      for (const common of ["success", "failure", "cancelled", "skipped", ""])
        for (const native of ["success", "failure", "cancelled", "skipped", ""]) {
          const operation = () => requireReleaseJobs(portable, common, native);
          let accepted = true;
          try {
            operation();
          } catch {
            accepted = false;
          }
          expect(accepted).toBe([portable, common, native].every((value) => value === "success"));
        }
  });
});

// The remaining tag cases run Git itself, including annotation type and reachability.
describe("real Git annotated tag semantics", () => {
  it("distinguishes a directly annotated commit from lightweight and off-main commits", async () => {
    const { inspectTag } = await import("./source.ts");
    const directory = mkdtempSync(join(tmpdir(), "lens-tag-"));
    const run = (...args: string[]) =>
      execFileSync("git", ["-c", "core.hooksPath=/dev/null", ...args], {
        cwd: directory,
        encoding: "utf8",
        env: {
          ...process.env,
          GIT_AUTHOR_NAME: "Fixture",
          GIT_AUTHOR_EMAIL: "fixture@example.test",
          GIT_COMMITTER_NAME: "Fixture",
          GIT_COMMITTER_EMAIL: "fixture@example.test",
        },
      }).trim();
    try {
      run("init", "-b", "main");
      for (const path of VERSION_FILES) {
        mkdirSync(dirname(join(directory, path)), { recursive: true });
        writeFileSync(join(directory, path), readFileSync(path));
      }
      const version = JSON.parse(readFileSync(join(directory, VERSION_FILES[0]), "utf8"))
        .version as string;
      writeFileSync(
        join(directory, ".release-please-manifest.json"),
        JSON.stringify({ ".": version }),
      );
      run("add", ".");
      run("commit", "-m", "chore: fixture");
      const main = run("rev-parse", "HEAD");
      run("update-ref", "refs/remotes/origin/main", main);
      const data = { schema: 1 as const, version, commit: main, pullRequest: 1 };
      run("tag", "-a", `v${version}`, "-m", annotation(data));
      expect(inspectTag(directory, `v${version}`)).toEqual(data);
      run("commit", "--allow-empty", "-m", "chore: main advanced");
      run("update-ref", "refs/remotes/origin/main", run("rev-parse", "HEAD"));
      expect(inspectTag(directory, `v${version}`)).toEqual(data);
      run("tag", "v0.0.1");
      expect(() => inspectTag(directory, "v0.0.1")).toThrow("Lightweight");
      run("checkout", "-b", "other");
      run("commit", "--allow-empty", "-m", "chore: off-main");
      const other = run("rev-parse", "HEAD");
      const offVersion = `${BigInt(version.split(".")[0]!) + 1n}.0.0`;
      run(
        "tag",
        "-a",
        `v${offVersion}`,
        "-m",
        annotation({ schema: 1, version: offVersion, commit: other, pullRequest: 1 }),
      );
      expect(() => inspectTag(directory, `v${offVersion}`)).toThrow(/.+/u);
      expect(readFileSync(join(directory, ".git/HEAD"), "utf8")).toContain("refs/heads/other");
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});
