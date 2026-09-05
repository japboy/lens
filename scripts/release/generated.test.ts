import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { expect, it } from "vitest";

it("accepts generated history and manifest while still checking current product text", () => {
  const root = mkdtempSync(join(tmpdir(), "lens-generated-release-"));
  const legacyName = ["Personal", "Lens"].join("");
  try {
    execFileSync("git", ["init", "--quiet", root]);
    mkdirSync(join(root, "mise-tasks/check"), { recursive: true });
    copyFileSync("mise-tasks/check/identity.ts", join(root, "mise-tasks/check/identity.ts"));
    copyFileSync("oxfmt.config.ts", join(root, "oxfmt.config.ts"));
    symlinkSync(resolve("node_modules"), join(root, "node_modules"), "dir");
    writeFileSync(join(root, ".gitignore"), "node_modules\n");
    writeFileSync(join(root, "CHANGELOG.md"), `# Changelog\n\n* feat: add ${legacyName}\n`);
    writeFileSync(join(root, ".release-please-manifest.json"), '{".":"0.1.0"}\n');
    const identity = () =>
      spawnSync(process.execPath, [join(root, "mise-tasks/check/identity.ts")], {
        cwd: root,
        encoding: "utf8",
      });
    const format = () =>
      spawnSync(resolve("node_modules/.bin/oxfmt"), ["--check", "."], {
        cwd: root,
        encoding: "utf8",
      });
    expect(identity().status).toBe(0);
    expect(format().status).toBe(0);
    writeFileSync(join(root, "README.md"), `# ${legacyName}\n`);
    expect(identity().stderr).toContain("README.md:1: uses a legacy product identity");
    writeFileSync(join(root, "README.md"), "# Lens\n");
    mkdirSync(join(root, "nested"));
    writeFileSync(join(root, "nested/CHANGELOG.md"), legacyName);
    expect(identity().stderr).toContain("nested/CHANGELOG.md:1: uses a legacy product identity");
    writeFileSync(join(root, "nested/.release-please-manifest.json"), '{".":"0.1.0"}');
    expect(format().status).not.toBe(0);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
