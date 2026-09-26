import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { parse } from "yaml";
import { VERIFICATION_REQUIREMENTS } from "./ci-plan.ts";
import { MACOS_BUILD_CACHE_INPUTS } from "./macos-build-contract.ts";
import { BUILD_VARIANTS } from "./workspace-policy.ts";

const quality = readFileSync(".github/workflows/code-quality.yml", "utf8");
const linux = readFileSync(".github/workflows/shared-rust-verification.yml", "utf8");
const native = readFileSync(".github/workflows/macos-verification.yml", "utf8");
const seed = readFileSync(".github/workflows/rust-cache-seed.yml", "utf8");
const qualityJobs = parse(quality).jobs;
const nativeSteps = parse(native).jobs["macos-verification"].steps;
const shell = qualityJobs["code-quality"].steps[0].run as string;

describe("actual workflow admission", () => {
  it("executes the actual aggregate shell for all 3,125 result combinations", () => {
    const outcomes = ["success", "failure", "cancelled", "skipped", ""];
    const cases: string[][] = [];
    const accepted: string[] = [];
    for (const requirements of VERIFICATION_REQUIREMENTS) {
      for (const repository of outcomes)
        for (const frontend of outcomes)
          for (const sharedRust of outcomes)
            for (const macos of outcomes) {
              const index = String(cases.length);
              cases.push([
                index,
                repository,
                frontend,
                sharedRust,
                macos,
                String(requirements.sharedRust),
                requirements.macos,
              ]);
              if (
                repository === "success" &&
                frontend === "success" &&
                sharedRust === (requirements.sharedRust ? "success" : "skipped") &&
                macos === (requirements.macos === "none" ? "skipped" : "success")
              )
                accepted.push(index);
            }
    }
    expect(cases).toHaveLength(3125);
    for (const [sharedRust, macos] of [
      ["", ""],
      ["false", "code"],
      ["false", "app"],
      ["false", "dmg"],
      ["true", ""],
      ["true", "unknown"],
      ["TRUE", "code"],
    ])
      cases.push([
        String(cases.length),
        "success",
        "success",
        "success",
        "success",
        sharedRust!,
        macos!,
      ]);
    // One shell session runs the unchanged workflow body in isolated subshells.
    // This avoids thousands of process-startup overheads in the Node test runner.
    const result = spawnSync(
      "bash",
      [
        "-c",
        `
      while IFS='|' read -r INDEX REPOSITORY_RESULT FRONTEND_RESULT SHARED_RUST_RESULT MACOS_RESULT SHARED_RUST MACOS; do
        if ( ${shell} ) >/dev/null 2>&1; then printf '%s\\n' "$INDEX"; fi
      done
    `,
      ],
      {
        input: cases.map((entry) => entry.join("|")).join("\n") + "\n",
        encoding: "utf8",
        timeout: 30_000,
      },
    );
    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    expect(result.stdout.trim().split("\n")).toEqual(accepted);
  }, 35_000);

  it("keeps independent repository/frontend owners before every Rust consumer", () => {
    const repository = qualityJobs["repository-verification"];
    const frontend = qualityJobs["frontend-verification"];
    expect(repository.needs).toBeUndefined();
    expect(frontend.needs).toBeUndefined();
    expect(
      repository.steps.findIndex(
        (step: { run?: string }) => step.run === "mise run verify:repository",
      ),
    ).toBeLessThan(
      repository.steps.findIndex(
        (step: { run?: string }) => step.run === "node scripts/ci-diff.ts",
      ),
    );
    expect(
      repository.steps.some(
        (step: { run?: string }) => step.run === "node scripts/release/cli.ts pr-title",
      ),
    ).toBe(true);
    expect(
      repository.steps.some(
        (step: { run?: string }) => step.run === "node scripts/release/cli.ts delta",
      ),
    ).toBe(true);
    for (const name of ["shared-rust-verification", "macos-verification"])
      expect(qualityJobs[name].needs).toEqual(["repository-verification", "frontend-verification"]);
    expect(qualityJobs["code-quality"].needs).toEqual([
      "repository-verification",
      "frontend-verification",
      "shared-rust-verification",
      "macos-verification",
    ]);
    expect(qualityJobs["code-quality"].if).toBe("always()");
    expect(quality).toContain("HEAD_SHA: ${{ github.sha }}");
    expect(quality).toContain("BASE_SHA: ${{ github.event.pull_request.base.sha }}");
    expect(quality).not.toContain("pull_request.head.sha");
    const verify = frontend.steps.findIndex(
      (step: { run?: string }) => step.run === "mise run verify:frontend",
    );
    const bind = frontend.steps.findIndex(
      (step: { run?: string }) => step.run === "node scripts/frontend-artifact.ts write",
    );
    const upload = frontend.steps.findIndex((step: { uses?: string }) =>
      step.uses?.startsWith("actions/upload-artifact@"),
    );
    expect(verify).toBeGreaterThanOrEqual(0);
    expect(bind).toBeGreaterThan(verify);
    expect(upload).toBeGreaterThan(bind);
    expect(frontend.steps[bind].if).toBeUndefined();
    expect(frontend.steps[upload].if).toBeUndefined();
    expect(frontend.steps[upload].with.name).toBe("frontend-${{ github.sha }}");
  });

  it("admits PR merge-ref writes and trusted main seeding while releases stay read-only", () => {
    const admission = nativeSteps.find((step: { id?: string }) => step.id === "cache-policy");
    const linuxSteps = parse(linux).jobs["shared-rust-verification"].steps;
    expect(linuxSteps.find((step: { id?: string }) => step.id === "cache-policy").run).toBe(
      admission.run,
    );
    const directory = mkdtempSync(join(tmpdir(), "lens-cache-authority-"));
    const output = join(directory, "output");
    const run = (overrides: Record<string, string>) => {
      writeFileSync(output, "");
      const result = spawnSync("bash", ["-e", "-c", admission.run], {
        env: {
          ...process.env,
          POLICY: "read-only",
          EVENT: "pull_request",
          REF: "refs/pull/129/merge",
          PR_NUMBER: "129",
          SOURCE_SHA: "",
          TAG: "",
          GITHUB_OUTPUT: output,
          ...overrides,
        },
        encoding: "utf8",
      });
      return { status: result.status, output: readFileSync(output, "utf8") };
    };
    try {
      expect(run({})).toEqual({ status: 0, output: "save=false\n" });
      expect(run({ POLICY: "pull-request" })).toEqual({ status: 0, output: "save=true\n" });
      for (const event of ["push", "workflow_dispatch"])
        expect(run({ POLICY: "main-seed", EVENT: event, REF: "refs/heads/main" })).toEqual({
          status: 0,
          output: "save=true\n",
        });
      expect(
        run({ EVENT: "workflow_dispatch", TAG: "v1.0.0", SOURCE_SHA: "a".repeat(40) }),
      ).toEqual({ status: 0, output: "save=false\n" });
      for (const policy of ["", "true", "main", "unknown"])
        expect(run({ POLICY: policy }).status).not.toBe(0);
      const invalidPrInputs: Record<string, string>[] = [
        { EVENT: "pull_request_target" },
        { EVENT: "push" },
        { EVENT: "workflow_dispatch" },
        { REF: "refs/heads/main" },
        { REF: "refs/pull/130/merge" },
        { REF: "refs/pull/129/head" },
        { PR_NUMBER: "" },
        { PR_NUMBER: "0" },
        { PR_NUMBER: "129x" },
        { SOURCE_SHA: "a".repeat(40) },
        { TAG: "v1.0.0" },
      ];
      for (const invalid of invalidPrInputs)
        expect(run({ POLICY: "pull-request", ...invalid }).status).not.toBe(0);
      const invalidSeedInputs: Record<string, string>[] = [
        { REF: "refs/heads/feature" },
        { EVENT: "pull_request" },
        { EVENT: "pull_request_target" },
        { SOURCE_SHA: "a".repeat(40) },
        { TAG: "v1.0.0" },
      ];
      for (const invalid of invalidSeedInputs)
        expect(
          run({ POLICY: "main-seed", REF: "refs/heads/main", EVENT: "push", ...invalid }).status,
        ).not.toBe(0);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
    for (const steps of [nativeSteps, linuxSteps]) {
      expect(steps[0].id).toBe("cache-policy");
      for (const cache of steps.filter((step: { uses?: string }) =>
        step.uses?.startsWith("Swatinem/rust-cache@"),
      )) {
        expect(cache.with["cache-on-failure"]).toBe(false);
        expect(cache.with["cache-workspace-crates"]).toBe(false);
      }
    }
  });

  it("separates immutable cache coverage and restores the other graph only on a miss", () => {
    const caches = nativeSteps.filter((step: { uses?: string }) =>
      step.uses?.startsWith("Swatinem/rust-cache@"),
    );
    expect(caches).toHaveLength(2);
    const [primary, fallback] = caches;
    expect(primary.id).toBe("rust-cache");
    expect(fallback.if).toBe(
      "steps.source.outputs.generation == 'current' && steps.rust-cache.outputs.cache-hit != 'true'",
    );
    expect(primary.with["save-if"]).toBe("${{ steps.cache-policy.outputs.save == 'true' }}");
    expect(fallback.with["save-if"]).toBe(false);
    expect(fallback.with["shared-key"]).toBe(
      "macos-aarch64-apple-darwin-${{ (inputs.seed_bundle_cache || inputs.verification_mode != 'code') && 'code-v1' || 'code-and-bundle-v1' }}-${{ steps.contract.outputs.cache_key }}",
    );
    expect(fallback.with.key).toBeUndefined();
    for (const property of [
      "prefix-key",
      "env-vars",
      "workspaces",
      "cache-targets",
      "cache-bin",
      "cache-on-failure",
      "cache-all-crates",
      "cache-workspace-crates",
    ])
      expect(fallback.with[property]).toEqual(primary.with[property]);
    expect(nativeSteps.indexOf(fallback)).toBeLessThan(
      nativeSteps.findIndex(
        (step: { name?: string }) =>
          step.name === "Run native verification without JavaScript dependencies",
      ),
    );
    // No commit/PR component: GitHub scopes saved caches to the merge ref.
    for (const cache of caches) {
      expect(cache.with["shared-key"]).not.toContain("github.sha");
      expect(cache.with["shared-key"]).not.toContain("pull_request.number");
    }
  });

  it("enforces modes and trusted seed capability using the actual admission shell", () => {
    const admission = nativeSteps.find(
      (step: { name?: string }) => step.name === "Validate native verification mode",
    ).run;
    const run = (overrides: Record<string, string>) =>
      spawnSync("bash", ["-e", "-c", admission], {
        env: {
          ...process.env,
          MODE: "code",
          ARTIFACT: "",
          TAG: "",
          SEED: "false",
          POLICY: "read-only",
          REF: "refs/pull/1/merge",
          ...overrides,
        },
        encoding: "utf8",
      }).status;
    expect(run({})).toBe(0);
    expect(run({ POLICY: "main-seed", REF: "refs/heads/main" })).not.toBe(0);
    for (const mode of ["app", "dmg"]) {
      expect(run({ MODE: mode, ARTIFACT: "frontend-sha" })).toBe(0);
      expect(run({ MODE: mode })).not.toBe(0);
    }
    for (const mode of ["", "none", "bundle", "unknown"])
      expect(run({ MODE: mode, ARTIFACT: "frontend-sha" })).not.toBe(0);
    expect(run({ MODE: "dmg", TAG: "v1.0.0", ARTIFACT: "frontend-sha" })).toBe(0);
    for (const mode of ["code", "app"])
      expect(run({ MODE: mode, TAG: "v1.0.0", ARTIFACT: "frontend-sha" })).not.toBe(0);
    const seedInput = {
      SEED: "true",
      POLICY: "main-seed",
      REF: "refs/heads/main",
      ARTIFACT: "frontend-sha",
    };
    expect(run(seedInput)).toBe(0);
    const invalidSeeds: Record<string, string>[] = [
      { POLICY: "read-only" },
      { REF: "refs/pull/1/merge" },
      { ARTIFACT: "" },
      { MODE: "app" },
      { MODE: "dmg" },
      { TAG: "v1.0.0" },
    ];
    for (const invalid of invalidSeeds) expect(run({ ...seedInput, ...invalid })).not.toBe(0);
  });

  it("admits complete historical release sources without downgrading partial or ordinary sources", () => {
    const admission = nativeSteps.find((step: { id?: string }) => step.id === "source").run;
    const linuxSteps = parse(linux).jobs["shared-rust-verification"].steps;
    expect(linuxSteps.find((step: { id?: string }) => step.id === "source").run).toBe(admission);
    const modernFiles = [
      "scripts/macos-build-contract.ts",
      "scripts/macos-bundle-build.ts",
      "mise-tasks/verify/macos-bundle.ts",
    ];
    const modernTasks = ["verify:macos", "verify:shared-rust", "build:macos-bundle"];
    const legacyFiles = ["mise-tasks/verify/bundle.ts"];
    const legacyTasks = ["verify:native", "verify:common"];
    const releaseEnv = {
      RELEASE_TAG: "v1.0.0",
      SOURCE_SHA: "a".repeat(40),
      CACHE_POLICY: "read-only",
      SEED_BUNDLE_CACHE: "false",
    };
    function admit(
      files: string[],
      tasks: string[],
      env: Record<string, string>,
    ): { status: number | null; output: string } {
      const directory = mkdtempSync(join(tmpdir(), "lens-source-capability-"));
      try {
        for (const file of files) {
          const path = join(directory, file);
          mkdirSync(dirname(path), { recursive: true });
          writeFileSync(path, "");
        }
        writeFileSync(
          join(directory, "mise.toml"),
          tasks.map((task) => `[tasks."${task}"]`).join("\n"),
        );
        const output = join(directory, "output");
        writeFileSync(output, "");
        const result = spawnSync("bash", ["-e", "-c", admission], {
          cwd: directory,
          encoding: "utf8",
          env: { ...process.env, ...env, GITHUB_OUTPUT: output },
        });
        return { status: result.status, output: readFileSync(output, "utf8") };
      } finally {
        rmSync(directory, { recursive: true, force: true });
      }
    }
    expect(
      admit(modernFiles, modernTasks, { ...releaseEnv, RELEASE_TAG: "", SOURCE_SHA: "" }),
    ).toEqual({ status: 0, output: "generation=current\n" });
    expect(admit(legacyFiles, legacyTasks, releaseEnv)).toEqual({
      status: 0,
      output: "generation=legacy\n",
    });
    for (const omit of modernFiles)
      expect(
        admit(
          modernFiles.filter((file) => file !== omit),
          modernTasks,
          releaseEnv,
        ).status,
      ).not.toBe(0);
    for (const omit of modernTasks)
      expect(
        admit(
          modernFiles,
          modernTasks.filter((task) => task !== omit),
          releaseEnv,
        ).status,
      ).not.toBe(0);
    expect(admit([], legacyTasks, releaseEnv).status).not.toBe(0);
    for (const omit of legacyTasks)
      expect(
        admit(
          legacyFiles,
          legacyTasks.filter((task) => task !== omit),
          releaseEnv,
        ).status,
      ).not.toBe(0);
    expect(
      admit([...modernFiles, ...legacyFiles], [...modernTasks, ...legacyTasks], releaseEnv).status,
    ).not.toBe(0);
    const invalidEnvironments: Record<string, string>[] = [
      { RELEASE_TAG: "" },
      { SOURCE_SHA: "" },
      { SOURCE_SHA: "main" },
      { CACHE_POLICY: "pull-request" },
      { SEED_BUNDLE_CACHE: "true" },
    ];
    for (const invalid of invalidEnvironments)
      expect(admit(legacyFiles, legacyTasks, { ...releaseEnv, ...invalid }).status).not.toBe(0);
    for (const steps of [nativeSteps, linuxSteps]) {
      const cache = steps.find((step: { uses?: string }) =>
        step.uses?.startsWith("Swatinem/rust-cache@"),
      );
      expect(cache.if).toBe("steps.source.outputs.generation == 'current'");
    }
    expect(nativeSteps.find((step: { id?: string }) => step.id === "contract").if).toBe(
      "steps.source.outputs.generation == 'current'",
    );
  });

  it("shares verified assets while keeping code and cache seeding independent of pnpm", () => {
    for (const workflow of [linux, native])
      expect(workflow).toContain("node scripts/frontend-artifact.ts check");
    expect(native).toContain("'verify:native' || 'verify:macos'");
    expect(linux).toContain("'verify:common' || 'verify:shared-rust'");
    expect(native).not.toContain("pnpm run build");
    const install = nativeSteps.find(
      (step: { run?: string }) => step.run === "pnpm install --frozen-lockfile",
    );
    expect(install.if).toBe("inputs.verification_mode != 'code'");
    const verifyAssets = nativeSteps.findIndex(
      (step: { run?: string }) => step.run === "node scripts/frontend-artifact.ts check",
    );
    const seedBuild = nativeSteps.findIndex(
      (step: { run?: string }) => step.run === "mise run build:macos-bundle",
    );
    expect(seedBuild).toBeGreaterThan(verifyAssets);
    expect(nativeSteps[seedBuild].if).toBe("inputs.seed_bundle_cache");
    expect(nativeSteps[verifyAssets].if).toBe(
      "inputs.verification_mode != 'code' || inputs.seed_bundle_cache",
    );
  });

  it("keys and seeds complete trusted cache contracts before restoring", () => {
    for (const workflow of [linux, native]) {
      expect(workflow).toContain("save-if: ${{ steps.cache-policy.outputs.save == 'true' }}");
      expect(workflow).toContain("cache-workspace-crates: false");
    }
    expect(quality.match(/cache_policy: pull-request/gu)).toHaveLength(2);
    expect(
      readFileSync(".github/workflows/release.yml", "utf8").match(/cache_policy: read-only/gu),
    ).toHaveLength(2);
    expect(seed.match(/cache_policy: main-seed/gu)).toHaveLength(2);
    // The pinned action ignores `key` whenever `shared-key` is set:
    // https://github.com/Swatinem/rust-cache/blob/6323deb102c322ba6fcbdcafc7e3dddab59af2b6/src/config.ts#L69-L86
    // Contract digests must therefore be part of the actual shared key.
    const linuxCache = parse(linux).jobs["shared-rust-verification"].steps.find(
      (step: { uses?: string }) => step.uses?.startsWith("Swatinem/rust-cache@"),
    ).with;
    const nativeCache = nativeSteps.find((step: { uses?: string }) =>
      step.uses?.startsWith("Swatinem/rust-cache@"),
    ).with;
    expect(linuxCache["shared-key"]).toBe(
      "common-x86_64-unknown-linux-gnu-${{ hashFiles('mise.lock', 'Cargo.toml') }}",
    );
    expect(nativeCache["shared-key"]).toBe(
      "macos-aarch64-apple-darwin-${{ (inputs.seed_bundle_cache || inputs.verification_mode != 'code') && 'code-and-bundle-v1' || 'code-v1' }}-${{ steps.contract.outputs.cache_key }}",
    );
    expect(linuxCache.key).toBeUndefined();
    expect(nativeCache.key).toBeUndefined();
    expect(native).toContain("env-vars: MACOSX_DEPLOYMENT_TARGET");
    expect(native.indexOf("node scripts/macos-build-contract.ts")).toBeLessThan(
      native.indexOf("uses: Swatinem/rust-cache@"),
    );
    const seedWorkflow = parse(seed);
    for (const path of MACOS_BUILD_CACHE_INPUTS) expect(seedWorkflow.on.push.paths).toContain(path);
    for (const name of ["macos-dependency-cache-seed", "linux-dependency-cache-seed"]) {
      expect(seedWorkflow.jobs[name].needs).toBe("frontend-verification");
      expect(seedWorkflow.jobs[name].with.frontend_artifact).toBe(
        "cache-frontend-${{ github.sha }}",
      );
      expect(seedWorkflow.jobs[name].if).toContain("github.ref == 'refs/heads/main'");
    }
    expect(seedWorkflow.jobs["macos-dependency-cache-seed"].with.seed_bundle_cache).toBe(true);
  });

  it("keeps packaged release linking distinct from the normal release check", () => {
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
