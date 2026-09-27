import { spawnSync } from "node:child_process";
import { cpSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";

it("runs the actual release bootstrap CLI without node_modules", () => {
  const root = mkdtempSync(join(tmpdir(), "lens-release-bootstrap-"));
  try {
    cpSync("scripts", join(root, "scripts"), { recursive: true });
    writeFileSync(join(root, "package.json"), '{"type":"module"}');
    const run = (mode: string, title: string) =>
      spawnSync(process.execPath, [join(root, "scripts/release/cli.ts"), mode], {
        cwd: root,
        encoding: "utf8",
        timeout: 10_000,
        env: { PATH: process.env.PATH, PR_TITLE: title },
      });
    const accepted = run("pr-title", "fix(release): validate source versions");
    expect(accepted.error).toBeUndefined();
    expect(accepted.stderr).toBe("");
    expect(accepted.status).toBe(0);
    const rejected = run("pr-title", "invalid title");
    expect(rejected.status).not.toBe(0);
    expect(rejected.stderr).toContain("PR title must be a Conventional Commit");
    // Promotion consumes artifact metadata, not source TOML. Reach its explicit
    // input validation without resolving optional parser dependencies at startup.
    const promotion = run("promote", "");
    expect(promotion.status).not.toBe(0);
    expect(promotion.stderr).toContain("Required explicit input: CONTROLLER_SHA");
    expect(promotion.stderr).not.toContain("MODULE_NOT_FOUND");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}, 30_000);
