#!/usr/bin/env node
//MISE description = "Build and inspect the native app or DMG using prebuilt frontend assets"
//MISE dir = "{{config_root}}"
//MISE wait_for = ["frontend:build", "verify:native"]

import { BUILD_PATHS } from "../../apps/desktop/tooling/build-paths.ts";
import { PAGE_ENTRIES } from "../../apps/desktop/src/page-entries.ts";
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { runVariant } from "../../scripts/run-workspace-variant.ts";
import { OUTPUT_SIDECAR_CONFIG, prepareOutputSidecar } from "../../scripts/output-sidecar.ts";
import { bundleContract, verifyApp, verifyDmg } from "../../scripts/release/bundle.ts";

const ROOT = fileURLToPath(new URL("../../", import.meta.url));
export function verifyNativeBundle(root = ROOT, kind = "app"): string {
  if (!["app", "dmg"].includes(kind)) throw new Error("Explicit app or dmg bundle kind required");
  if (process.platform !== "darwin" || process.arch !== "arm64")
    throw new Error("Bundle verification requires the admitted Apple-silicon host");
  const application = join(root, "apps/desktop");
  const contract = bundleContract(root);
  for (const entry of Object.values(PAGE_ENTRIES)) {
    if (!existsSync(join(application, BUILD_PATHS.webview, entry)))
      throw new Error(`Prebuilt frontend entry is required: ${entry}`);
  }
  const directory = join(root, "target/aarch64-apple-darwin/release/bundle");
  const previous = process.env.MACOSX_DEPLOYMENT_TARGET;
  try {
    process.env.MACOSX_DEPLOYMENT_TARGET = contract.minimum;
    prepareOutputSidecar(root, "aarch64-apple-darwin", "release");
    runVariant("macos-bundle-build", root);
    // Remove only generated packaging output; stale DMGs cannot become candidates.
    rmSync(directory, { recursive: true, force: true });
    // Resolve under the installed pnpm environment before setting CI for Tauri only.
    // pnpm changes its virtual-store policy when CI changes after installation.
    const cli = execFileSync(
      "pnpm",
      ["exec", "node", "-p", 'require.resolve("@tauri-apps/cli/tauri.js")'],
      { cwd: application, encoding: "utf8" },
    ).trim();
    execFileSync(
      process.execPath,
      [
        cli,
        "bundle",
        "--target",
        "aarch64-apple-darwin",
        "--features",
        "tauri/custom-protocol",
        "--bundles",
        kind,
        "--ci",
        "--config",
        OUTPUT_SIDECAR_CONFIG,
        ...(kind === "dmg" ? ["--config", "src-tauri/tauri.release.conf.json"] : []),
      ],
      { cwd: application, stdio: "inherit", env: { ...process.env, CI: "true" } },
    );
  } finally {
    if (previous === undefined) delete process.env.MACOSX_DEPLOYMENT_TARGET;
    else process.env.MACOSX_DEPLOYMENT_TARGET = previous;
  }
  const app = join(directory, "macos/Lens.app");
  // Tauri removes its intermediate .app when only DMG was requested.
  if (kind === "app") verifyApp(app, contract);
  const bundle =
    kind === "app" ? app : join(directory, "dmg", `Lens_${contract.version}_aarch64.dmg`);
  if (kind === "dmg") {
    const candidates = readdirSync(join(directory, "dmg")).filter((name) => name.endsWith(".dmg"));
    if (JSON.stringify(candidates) !== JSON.stringify([`Lens_${contract.version}_aarch64.dmg`]))
      throw new Error("Unexpected DMG candidate set");
    verifyDmg(bundle, contract);
  }
  process.stdout.write(
    `${JSON.stringify({ bundle, ...contract, architecture: "arm64", applicationSignature: "adhoc", dmgSignature: kind === "dmg" ? "unsigned" : "not-applicable", notarization: "not-performed", status: "passed" })}\n`,
  );
  return bundle;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length > 3) throw new Error("Only an explicit bundle kind may be supplied");
  verifyNativeBundle(ROOT, process.argv[2] ?? "app");
}
