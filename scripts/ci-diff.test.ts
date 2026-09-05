import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { planGitChanges } from "./ci-diff.ts";

function fixture(
  work: (
    root: string,
    git: (args: string[]) => string,
    write: (path: string, content: string | Buffer) => void,
  ) => void,
) {
  const root = mkdtempSync(join(tmpdir(), "lens-ci-diff-"));
  const git = (args: string[]) =>
    execFileSync(
      "git",
      [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "user.name=CI fixture",
        "-c",
        "user.email=fixture@example.test",
        ...args,
      ],
      { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    ).trim();
  const write = (path: string, content: string | Buffer) => {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  };
  try {
    git(["init", "-b", "main"]);
    write("packages/usecase/src/state.rs", "pub fn value() -> u8 { 1 }\n");
    write("README.md", "fixture\n");
    git(["add", "."]);
    git(["commit", "-m", "fixture baseline"]);
    work(root, git, write);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
const commit = (git: (args: string[]) => string) => {
  git(["add", "."]);
  git(["commit", "-m", "fixture change"]);
  return git(["rev-parse", "HEAD"]);
};

describe("actual tested Git snapshot classification", () => {
  it("reads exact committed before/after bodies and preserves the source checkout", () =>
    fixture((root, git, write) => {
      const base = git(["rev-parse", "HEAD"]);
      write("packages/usecase/src/state.rs", "pub fn value() -> u8 { 2 }\n");
      const head = commit(git);
      expect(planGitChanges(root, base, head)).toEqual({
        base,
        head,
        plan: "portable-rust",
        paths: ["packages/usecase/src/state.rs"],
      });
      expect(git(["status", "--porcelain"])).toBe("");
      expect(readFileSync(join(root, "packages/usecase/src/state.rs"), "utf8")).toContain("{ 2 }");
    }));

  it("does not decode binary resources and emits both sides of a rename", () =>
    fixture((root, git, write) => {
      write("packages/adapter-platform-macos/native/fixture.dat", Buffer.from([0xff, 0x00, 0xfe]));
      const base = commit(git);
      mkdirSync(join(root, "apps/desktop/public"), { recursive: true });
      renameSync(
        join(root, "packages/adapter-platform-macos/native/fixture.dat"),
        join(root, "apps/desktop/public/fixture.png"),
      );
      const head = commit(git);
      expect(planGitChanges(root, base, head)).toMatchObject({
        plan: "native-bundle",
        paths: [
          "apps/desktop/public/fixture.png",
          "packages/adapter-platform-macos/native/fixture.dat",
        ],
      });
    }));

  it("rejects refs, a different checkout and modified or untracked source", () =>
    fixture((root, git, write) => {
      const base = git(["rev-parse", "HEAD"]);
      write("README.md", "changed\n");
      const head = commit(git);
      expect(() => planGitChanges(root, "main", head)).toThrow("Exact base");
      expect(() => planGitChanges(root, base, base)).toThrow("actual tested checkout");
      write("README.md", "dirty\n");
      expect(() => planGitChanges(root, base, head)).toThrow("modified source checkout");
      write("README.md", "changed\n");
      write("untracked.rs", "fn x() {}\n");
      expect(() => planGitChanges(root, base, head)).toThrow("modified source checkout");
    }));

  it("rejects a symlink instead of treating its portable-looking destination as source", () =>
    fixture((root, git) => {
      const base = git(["rev-parse", "HEAD"]);
      mkdirSync(join(root, "apps/desktop/src"), { recursive: true });
      symlinkSync("../../../README.md", join(root, "apps/desktop/src/component.ts"));
      const head = commit(git);
      expect(() => planGitChanges(root, base, head)).toThrow("Unreviewed changed object");
    }));

  it("requires native verification for a valid empty diff", () =>
    fixture((root, git) => {
      const head = git(["rev-parse", "HEAD"]);
      expect(planGitChanges(root, head, head).plan).toBe("full");
    }));
});
