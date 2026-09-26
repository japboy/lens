import { createHash } from "node:crypto";
import { appendFileSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { bundleContract } from "./release/bundle.ts";

// A capability change must produce a new cache even when dependency versions match.
export const MACOS_BUILD_CACHE_CAPABILITY = "native-code-and-bundle-v1";
export const MACOS_BUILD_CACHE_INPUTS = [
  "Cargo.toml",
  "mise.toml",
  "mise.lock",
  "package.json",
  "scripts/macos-build-contract.ts",
  "scripts/macos-bundle-build.ts",
  "scripts/workspace-policy.ts",
  "scripts/run-workspace-variant.ts",
  "scripts/release/bundle.ts",
  "scripts/release/version.ts",
  "mise-tasks/inspect/features.ts",
  "mise-tasks/verify/macos-bundle.ts",
  "apps/desktop/src-tauri/tauri.conf.json",
  "apps/desktop/src-tauri/tauri.macos.conf.json",
  "apps/desktop/src-tauri/tauri.release.conf.json",
] as const;

export function macosBuildContract(root: string) {
  const { minimum } = bundleContract(root);
  const digest = createHash("sha256").update(MACOS_BUILD_CACHE_CAPABILITY).update("\0");
  for (const path of MACOS_BUILD_CACHE_INPUTS)
    digest
      .update(path)
      .update("\0")
      .update(readFileSync(join(root, path)))
      .update("\0");
  return {
    cacheKey: `${MACOS_BUILD_CACHE_CAPABILITY}-${digest.digest("hex")}`,
    deploymentTarget: minimum,
  };
}

export function publishMacosBuildContract(root: string, environment: NodeJS.ProcessEnv): void {
  const contract = macosBuildContract(root);
  if (environment.GITHUB_OUTPUT)
    appendFileSync(
      environment.GITHUB_OUTPUT,
      `cache_key=${contract.cacheKey}\ndeployment_target=${contract.deploymentTarget}\n`,
    );
  if (environment.GITHUB_ENV)
    appendFileSync(
      environment.GITHUB_ENV,
      `MACOSX_DEPLOYMENT_TARGET=${contract.deploymentTarget}\n`,
    );
  process.stdout.write(`${JSON.stringify(contract)}\n`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length !== 2) throw new Error("The macOS build contract accepts no arguments");
  publishMacosBuildContract(fileURLToPath(new URL("..", import.meta.url)), process.env);
}
