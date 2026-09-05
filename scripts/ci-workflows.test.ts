import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { CI_PLANS } from "./ci-plan.ts";
import { BUILD_VARIANTS } from "./workspace-policy.ts";

const quality = readFileSync(".github/workflows/code-quality.yml", "utf8");
const linux = readFileSync(".github/workflows/linux-common.yml", "utf8");
const native = readFileSync(".github/workflows/native-quality.yml", "utf8");
const seed = readFileSync(".github/workflows/rust-cache-seed.yml", "utf8");
const shell = quality
  .split("      - name: Require the complete verification state\n")[1]!
  .split("        run: |\n")[1]!
  .split("\n")
  .map((line) => line.replace(/^ {10}/u, ""))
  .join("\n");

describe("actual workflow admission", () => {
  it("executes the actual aggregate shell for every runner result tuple", () => {
    const outcomes = ["success", "failure", "cancelled", "skipped", ""];
    for (const [plan, expected] of Object.entries(CI_PLANS)) {
      for (const portable of outcomes)
        for (const common of outcomes)
          for (const mac of outcomes) {
            const result = spawnSync("bash", ["-c", shell], {
              encoding: "utf8",
              timeout: 1000,
              env: {
                ...process.env,
                PLAN: plan,
                PORTABLE_RESULT: portable,
                COMMON_RESULT: common,
                NATIVE_RESULT: mac,
              },
            });
            const accepted =
              portable === "success" &&
              common === (expected.common ? "success" : "skipped") &&
              mac === (expected.native === "none" ? "skipped" : "success");
            expect(result.status === 0).toBe(accepted);
          }
    }
    const missing = spawnSync("bash", ["-c", shell], {
      env: {
        ...process.env,
        PLAN: "unknown",
        PORTABLE_RESULT: "success",
        COMMON_RESULT: "success",
        NATIVE_RESULT: "success",
      },
    });
    expect(missing.status).not.toBe(0);
  }, 30_000);

  it("classifies the tested merge commit after portable verification", () => {
    expect(quality).toContain("HEAD_SHA: ${{ github.sha }}");
    expect(quality).toContain("BASE_SHA: ${{ github.event.pull_request.base.sha }}");
    expect(quality.indexOf("mise run verify:portable")).toBeLessThan(
      quality.indexOf("node scripts/ci-diff.ts"),
    );
    expect(quality).not.toContain("pull_request.head.sha");
    expect(quality).toContain('["portable-rust","native-code","native-bundle","full"]');
    expect(quality).toContain('["native-code","native-bundle","full"]');
  });

  it("shares source-bound artifacts and keeps ordinary native checks independent of pnpm", () => {
    expect(quality).toContain("name: frontend-${{ github.sha }}");
    expect(quality).toContain("node scripts/frontend-artifact.ts write");
    for (const workflow of [linux, native])
      expect(workflow).toContain("node scripts/frontend-artifact.ts check");
    expect(native).toContain(
      "if: inputs.native_mode == 'bundle'\n        run: pnpm install --frozen-lockfile",
    );
    expect(native).toContain("run: mise run verify:native");
    expect(native).not.toContain("pnpm run build");
    expect(linux).toContain("run: mise run verify:common");
  });

  it("keeps cache writes on trusted main and separates target/variant cache domains", () => {
    for (const workflow of [linux, native]) {
      expect(workflow).toContain(
        "save-if: ${{ inputs.save_cache && github.ref == 'refs/heads/main' }}",
      );
      expect(workflow).toContain("cache-workspace-crates: false");
      expect(workflow).toContain("hashFiles('mise.lock', 'scripts/workspace-variants.json')");
    }
    expect(quality.match(/save_cache: false/gu)).toHaveLength(2);
    expect(linux).toContain("shared-key: common-x86_64-unknown-linux-gnu");
    expect(native).toContain("shared-key: native-aarch64-apple-darwin");
    expect(seed).toContain("linux-dependency-cache-seed:");
  });

  it("declares packaged-mode release linking separately from ordinary normal/test variants", () => {
    expect(BUILD_VARIANTS.find((variant) => variant.id === "macos-bundle-build")).toMatchObject({
      operation: "build",
      profile: "release",
      features: ["tauri/custom-protocol"],
      targets: "lib-and-bins",
    });
    expect(
      BUILD_VARIANTS.find((variant) => variant.id === "macos-production-release")!.features,
    ).toEqual([]);
  });
});
