import { BUILD_PATHS } from "../apps/desktop/tooling/build-paths.ts";
import { PAGE_ENTRIES } from "../apps/desktop/src/page-entries.ts";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { frontendArtifact, frontendFiles } from "./frontend-artifact.ts";
import { generationFiles } from "../apps/desktop/tooling/prerender/verify.ts";
import { sourceDigest, sourceInputs } from "../apps/desktop/tooling/prerender/source.ts";

function seal(root: string, assets: string): void {
  writeFileSync(
    join(assets, "generation.json"),
    JSON.stringify({
      generation: sourceDigest(sourceInputs(root)),
      files: generationFiles(assets),
    }),
  );
}

function page(view: string): string {
  return `<lens-${view}-view defer-hydration><template shadowrootmode="open"><!--lit-part fixture--><main>fixture</main><!--/lit-part--></template></lens-${view}-view>`;
}

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
    writeFileSync(join(root, ".gitignore"), "/target/\n/apps/desktop/.build/\n");
    git(["add", ".gitignore"]);
    git(["commit", "-qm", "fixture"]);
    const assets = join(root, "apps/desktop", BUILD_PATHS.webview);
    mkdirSync(assets, { recursive: true });
    for (const [view, entry] of Object.entries(PAGE_ENTRIES))
      writeFileSync(join(assets, entry), page(view));
    mkdirSync(join(assets, ".vite"));
    writeFileSync(join(assets, ".vite/manifest.json"), "{}");
    seal(root, assets);
    work(root, assets);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe("same-source frontend artifact integrity", () => {
  it("rejects incomplete DSD even if the generation manifest was resealed", () =>
    fixture((root, assets) => {
      writeFileSync(join(assets, "about.html"), "<main>client-only</main>");
      seal(root, assets);
      expect(() => frontendArtifact("write", root)).toThrow("Invalid generated DSD");
    }));
  it("rejects missing initial assets and mixed development generations", () =>
    fixture((root, assets) => {
      writeFileSync(join(assets, "about.html"), page("about") + '<img src="/assets/missing.png">');
      seal(root, assets);
      expect(() => frontendArtifact("write", root)).toThrow("Missing generated asset");
      writeFileSync(
        join(assets, "about.html"),
        page("about") + '<script src="/_generations/old/entry.js"></script>',
      );
      seal(root, assets);
      expect(() => frontendArtifact("write", root)).toThrow("Mixed generation URL");
    }));
  it("rejects stale source generations and altered files before binding", () =>
    fixture((root, assets) => {
      const path = join(assets, "generation.json");
      const metadata = JSON.parse(readFileSync(path, "utf8"));
      metadata.generation = "0".repeat(64);
      writeFileSync(path, JSON.stringify(metadata));
      expect(() => frontendArtifact("write", root)).toThrow("source does not match");
      seal(root, assets);
      writeFileSync(join(assets, "extra.js"), "stale");
      expect(() => frontendArtifact("write", root)).toThrow("file set or digest does not match");
    }));
  it.each(Object.values(PAGE_ENTRIES))("rejects a build missing %s", (entry) =>
    fixture((_root, assets) => {
      rmSync(join(assets, entry));
      expect(() => frontendFiles(assets)).toThrow(entry);
    }),
  );
  it("checks the complete source-bound file set, including reserved property names", () =>
    fixture((root, assets) => {
      writeFileSync(join(assets, "__proto__"), "asset");
      expect(Object.hasOwn(frontendFiles(assets), "__proto__")).toBe(true);
      seal(root, assets);
      frontendArtifact("write", root);
      expect(() => frontendArtifact("check", root)).not.toThrow();
    }));
  it("rejects missing, modified and extra asset contents", () =>
    fixture((root, assets) => {
      frontendArtifact("write", root);
      writeFileSync(join(assets, "about.html"), "modified");
      expect(() => frontendArtifact("check", root)).toThrow("does not match");
      for (const [view, entry] of Object.entries(PAGE_ENTRIES))
        writeFileSync(join(assets, entry), page(view));
      writeFileSync(join(assets, "extra.js"), "extra");
      expect(() => frontendArtifact("check", root)).toThrow("does not match");
      rmSync(join(assets, "about.html"));
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
