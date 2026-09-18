import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { assertReleaseDelta, releasePath, snapshotReleaseSource } from "./release-delta.ts";
import type { ReleaseFile } from "./release-delta.ts";
import { VERSION_FILES } from "./version.ts";

const root = new URL("../../", import.meta.url);
const temporary: string[] = [];
afterEach(() => {
  for (const path of temporary.splice(0)) rmSync(path, { recursive: true, force: true });
});
const file = (content: string): ReleaseFile => ({ mode: "100644", content: Buffer.from(content) });
function fixture() {
  const baseline = new Map<string, ReleaseFile>(
    [...VERSION_FILES, "CHANGELOG.md"].map((path) => [
      path,
      file(readFileSync(new URL(path, root), "utf8")),
    ]),
  );
  baseline.set("src/unchanged.bin", { mode: "100644", content: Buffer.from([0, 255]) });
  const before = JSON.parse(baseline.get("package.json")!.content.toString()).version as string;
  const version = before.replace(/\d+$/u, (patch) => String(Number(patch) + 1));
  const candidate = new Map(baseline);
  for (const path of VERSION_FILES) {
    let content = baseline.get(path)!.content.toString();
    if (path === "Cargo.lock")
      content = content
        .split("[[package]]")
        .map((entry) =>
          /^source = /mu.test(entry)
            ? entry
            : entry.replace(/^version = "[^\n"]+"$/mu, `version = "${version}"`),
        )
        .join("[[package]]");
    else if (path === "Cargo.toml")
      content = content.replace(`version = "${before}"`, `version = "${version}"`);
    else if (path.endsWith(".json")) {
      const value = JSON.parse(content) as Record<string, unknown>;
      value[path === ".release-please-manifest.json" ? "." : "version"] = version;
      content = JSON.stringify(value, null, 2) + "\n";
    }
    candidate.set(path, file(content));
  }
  const changelog = baseline.get("CHANGELOG.md")!.content.toString();
  candidate.set(
    "CHANGELOG.md",
    file(
      changelog.replace(/^## /mu, `## ${version}\n\n### Bug Fixes\n\n* Correct behavior.\n\n## `),
    ),
  );
  return { baseline, candidate, version };
}

describe("release semantic delta", () => {
  it("admits formatting changes while preserving all non-version values and changelog history", () => {
    const { baseline, candidate, version } = fixture();
    expect(assertReleaseDelta(baseline, candidate).version).toBe(version);
  });

  it.each(["package.json", "apps/desktop/src-tauri/tauri.conf.json", "Cargo.toml"])(
    "rejects latest-base non-version rollback in %s",
    (path) => {
      const { baseline, candidate } = fixture();
      if (path.endsWith(".json")) {
        const value = JSON.parse(baseline.get(path)!.content.toString()) as Record<string, unknown>;
        value.newMainValue = true;
        baseline.set(path, file(JSON.stringify(value)));
      } else
        baseline.set(
          path,
          file(baseline.get(path)!.content + "\n[workspace.metadata]\nnew_main_value = true\n"),
        );
      expect(() => assertReleaseDelta(baseline, candidate)).toThrow("non-version manifest data");
    },
  );

  it.each(["unowned", "removed", "new", "mode", "member", "history", "preamble", "no-changelog"])(
    "rejects %s changes",
    (kind) => {
      const { baseline, candidate } = fixture();
      if (kind === "unowned") candidate.set("src/unchanged.bin", file("changed"));
      if (kind === "removed") candidate.delete("package.json");
      if (kind === "new") candidate.set("extra.json", file("{}"));
      if (kind === "mode")
        candidate.set("package.json", { ...candidate.get("package.json")!, mode: "100755" });
      if (kind === "member")
        candidate.set(
          "packages/domain/Cargo.toml",
          file(candidate.get("packages/domain/Cargo.toml")!.content + "\n# unowned change\n"),
        );
      if (kind === "history")
        candidate.set(
          "CHANGELOG.md",
          file(candidate.get("CHANGELOG.md")!.content + "changed old history\n"),
        );
      if (kind === "preamble")
        candidate.set(
          "CHANGELOG.md",
          file("changed preamble\n" + candidate.get("CHANGELOG.md")!.content),
        );
      if (kind === "no-changelog") candidate.set("CHANGELOG.md", baseline.get("CHANGELOG.md")!);
      expect(() => assertReleaseDelta(baseline, candidate)).toThrow(/release|Release/u);
    },
  );

  it.each([
    "../Cargo.toml",
    "/Cargo.toml",
    "a/../Cargo.toml",
    "a\\Cargo.toml",
    "a//b",
    "./Cargo.toml",
    "a\0b",
  ])("rejects unsafe path %j", (path) => {
    expect(() => releasePath(path)).toThrow("contained repository path");
  });

  it("rejects Git symlinks before archive extraction", () => {
    const directory = mkdtempSync(join(tmpdir(), "lens-delta-test-"));
    temporary.push(directory);
    const git = (...args: string[]) =>
      execFileSync("git", args, {
        cwd: directory,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      }).trim();
    git("init", "-q");
    git("config", "user.email", "test@example.invalid");
    git("config", "user.name", "Fixture");
    writeFileSync(join(directory, "source"), "fixed");
    symlinkSync("/tmp", join(directory, "link"));
    git("add", ".");
    git("-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture");
    expect(() =>
      snapshotReleaseSource(directory, git("rev-parse", "HEAD"), join(directory, "archive")),
    ).toThrow("without links or submodules");
  });
});
