import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { VERSION_FILES } from "./version.ts";
import { describe, expect, it } from "vitest";
import type { Request } from "./github.ts";
import { previousRelease } from "./control.ts";
import type { Release } from "./control.ts";
import { annotation, assertPrTitle, changelogSection } from "./source.ts";
import { sha256, verifyArtifact } from "./artifact.ts";
import type { ReleaseManifest } from "./artifact.ts";

const source = "a".repeat(40);
const identity = { tag: "v0.1.0", source, repository: "owner/repo", runId: "123" };
describe("release history and source syntax", () => {
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
    for (const name of ["v1.0.0-rc.1", "v2.0.0+build.1", "version-backup", "v01.0.0", "other"])
      tags.push({ name });
    releases.push({ tag_name: "v1.0.0-rc.1", draft: false, prerelease: true });
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
  return {
    directory,
    manifest,
    cleanup: () => rmSync(directory, { recursive: true, force: true }),
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
