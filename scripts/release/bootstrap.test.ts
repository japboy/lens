import { spawnSync } from "node:child_process";
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";
import { MACOS_BUILD_CACHE_INPUTS } from "../macos-build-contract.ts";
import { VERSION_FILES } from "./version.ts";
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

it("runs the macOS contract CLI with its pinned source parser installed", () => {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "lens-macos-bootstrap-")));
  try {
    cpSync("scripts", join(root, "scripts"), { recursive: true });
    for (const path of new Set([...MACOS_BUILD_CACHE_INPUTS, ...VERSION_FILES])) {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      cpSync(path, join(root, path));
    }
    const sdk = join(root, "sdk-fixture.mjs");
    writeFileSync(
      sdk,
      `
      import childProcess from "node:child_process";
      import { syncBuiltinESMExports } from "node:module";
      childProcess.execFileSync = (command, args) => {
        if (command === "/usr/bin/xcrun" && args.join(" ") === "--sdk macosx --show-sdk-version") return "27.0";
        throw new Error("Unexpected bootstrap subprocess");
      };
      syncBuiltinESMExports();
    `,
    );
    const run = () =>
      spawnSync(
        process.execPath,
        ["--import", sdk, join(root, "scripts/macos-build-contract.ts")],
        {
          cwd: root,
          encoding: "utf8",
          timeout: 10_000,
          env: { PATH: process.env.PATH },
        },
      );
    const missing = run();
    expect(missing.status).not.toBe(0);
    expect(missing.stderr).toContain("Cannot find module '@iarna/toml'");
    const parser = dirname(createRequire(import.meta.url).resolve("@iarna/toml/package.json"));
    expect(JSON.parse(readFileSync(join(parser, "package.json"), "utf8")).version).toBe(
      JSON.parse(readFileSync("package.json", "utf8")).devDependencies["@iarna/toml"],
    );
    cpSync(parser, join(root, "node_modules/@iarna/toml"), { recursive: true });
    const installed = run();
    expect(installed.error).toBeUndefined();
    expect(installed.stderr).toBe("");
    expect(installed.status).toBe(0);
    expect(JSON.parse(installed.stdout)).toMatchObject({ deploymentTarget: "15.2" });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}, 30_000);
