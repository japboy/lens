import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { frontendArtifact, frontendFiles } from "./frontend-artifact.ts";

function fixture(work: (root: string, assets: string) => void) {
  const root = mkdtempSync(join(tmpdir(), "lens-frontend-artifact-"));
  const git = (args: string[]) =>
    execFileSync(
      "git",
      [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "user.name=Artifact fixture",
        "-c",
        "user.email=fixture@example.test",
        ...args,
      ],
      { cwd: root, stdio: "pipe" },
    );
  try {
    git(["init", "-q"]);
    writeFileSync(join(root, ".gitignore"), "/target/\n/apps/desktop/dist/\n");
    git(["add", ".gitignore"]);
    git(["commit", "-qm", "fixture"]);
    const assets = join(root, "apps/desktop/dist");
    mkdirSync(assets, { recursive: true });
    writeFileSync(join(assets, "index.html"), "<main>fixture</main>");
    work(root, assets);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe("same-source frontend artifact integrity", () => {
  it("checks the complete source-bound file set, including reserved property names", () =>
    fixture((root, assets) => {
      writeFileSync(join(assets, "__proto__"), "asset");
      expect(Object.hasOwn(frontendFiles(assets), "__proto__")).toBe(true);
      frontendArtifact("write", root);
      expect(() => frontendArtifact("check", root)).not.toThrow();
    }));
  it("rejects missing, modified and extra asset contents", () =>
    fixture((root, assets) => {
      frontendArtifact("write", root);
      writeFileSync(join(assets, "index.html"), "modified");
      expect(() => frontendArtifact("check", root)).toThrow("does not match");
      writeFileSync(join(assets, "index.html"), "<main>fixture</main>");
      writeFileSync(join(assets, "extra.js"), "extra");
      expect(() => frontendArtifact("check", root)).toThrow("does not match");
      rmSync(join(assets, "index.html"));
      expect(() => frontendArtifact("check", root)).toThrow("entry asset is missing");
    }));
  it("rejects a different source commit and dirty source state", () =>
    fixture((root) => {
      frontendArtifact("write", root);
      const path = join(root, "target/ci/frontend-manifest.json");
      const manifest = JSON.parse(readFileSync(path, "utf8"));
      manifest.source = "0".repeat(40);
      writeFileSync(path, JSON.stringify(manifest));
      expect(() => frontendArtifact("check", root)).toThrow("does not match");
      writeFileSync(join(root, "untracked"), "source");
      expect(() => frontendArtifact("write", root)).toThrow("clean source checkout");
    }));
  it("rejects symlinks and unspecified modes", () =>
    fixture((root, assets) => {
      expect(() => frontendArtifact("unknown", root)).toThrow("explicit frontend artifact mode");
      symlinkSync("../../../.gitignore", join(assets, "escape"));
      expect(() => frontendArtifact("write", root)).toThrow("only regular files");
    }));
});
