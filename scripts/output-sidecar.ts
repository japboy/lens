import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const OUTPUT_SIDECAR_CONFIG = "src-tauri/tauri.output.conf.json";

export function sidecarBuildPlan(target: string, profile: "dev" | "release") {
  if (!/^(?:aarch64|x86_64)-(?:apple-darwin|unknown-linux-gnu|pc-windows-msvc)$/.test(target))
    throw new Error(`Unsupported output sidecar target: ${target}`);
  const extension = target.includes("windows") ? ".exe" : "";
  return {
    arguments: [
      "build",
      "--locked",
      "--package",
      "adapter-output-mcp",
      "--bin",
      "lens-output-mcp",
      "--target",
      target,
      "--profile",
      profile,
    ],
    executable: `lens-output-mcp${extension}`,
    stagedName: `lens-output-mcp-${target}${extension}`,
    profileDirectory: profile === "dev" ? "debug" : "release",
  };
}

/** Explicit prebuild for Tauri entrypoints; normal Cargo checks never require staged binaries. */
export function prepareOutputSidecar(
  root: string,
  target: string,
  profile: "dev" | "release",
): void {
  const plan = sidecarBuildPlan(target, profile);
  execFileSync("cargo", plan.arguments, { cwd: root, stdio: "inherit" });
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"], {
      cwd: root,
      encoding: "utf8",
    }),
  ) as { target_directory: string };
  const destination = join(root, "apps/desktop/src-tauri/binaries", plan.stagedName);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(
    join(metadata.target_directory, target, plan.profileDirectory, plan.executable),
    destination,
  );
}

export function tauriOutputArguments(args: readonly string[]): {
  arguments: string[];
  profile?: "dev" | "release";
  target?: string;
} {
  const separator = args.indexOf("--");
  const options = separator < 0 ? args : args.slice(0, separator);
  if (
    !["dev", "build", "bundle"].includes(args[0] ?? "") ||
    options.includes("--help") ||
    options.includes("-h")
  )
    return { arguments: [...args] };
  if (options.some((arg) => arg === "--profile" || arg.startsWith("--profile=")))
    throw new Error("Custom Tauri profiles require an explicit sidecar build contract.");
  const targetIndex = options.findIndex((arg) => arg === "--target" || arg === "-t");
  const target =
    targetIndex >= 0
      ? options[targetIndex + 1]
      : options.find((arg) => arg.startsWith("--target="))?.slice(9);
  if (targetIndex >= 0 && !target) throw new Error("--target requires a target triple.");
  return {
    arguments: [args[0]!, "--config", OUTPUT_SIDECAR_CONFIG, ...args.slice(1)],
    profile:
      args[0] === "dev" || options.includes("--debug") || options.includes("-d")
        ? "dev"
        : "release",
    ...(target ? { target } : {}),
  };
}

function main(): void {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const app = join(root, "apps/desktop");
  const plan = tauriOutputArguments(process.argv.slice(2));
  if (plan.profile) {
    const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (.+)$/m)?.[1];
    if (!host) throw new Error("Cannot determine the Rust host target.");
    prepareOutputSidecar(root, plan.target ?? host, plan.profile);
  }
  const require = createRequire(join(app, "package.json"));
  const result = spawnSync(
    process.execPath,
    [require.resolve("@tauri-apps/cli/tauri.js"), ...plan.arguments],
    { cwd: app, stdio: "inherit" },
  );
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
