import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { bundleContract } from "./release/bundle.ts";
import { parseToml, tomlTable } from "./release/toml.ts";

// Bump this when a coverage change requires a fresh immutable dependency cache.
export const MACOS_DEPENDENCY_CACHE_REVISION = "native-dependency-compat-v2";

// Swatinem hashes the Rust environment and member manifests; the virtual
// workspace root also controls resolution, inherited settings and profiles.
function cacheManifest(root: string) {
  const manifest = parseToml(readFileSync(join(root, "Cargo.toml"), "utf8"), "Cargo.toml");
  const workspace = tomlTable(manifest.workspace, "Cargo.toml: workspace");
  const workspacePackage = tomlTable(workspace.package, "Cargo.toml: workspace.package");
  delete workspacePackage.version;
  return manifest;
}

function canonicalToml(_key: string, value: unknown) {
  if (typeof value === "bigint") return value.toString();
  if (value === null || typeof value !== "object" || Array.isArray(value)) return value;
  const table = value as Record<string, unknown>;
  return Object.fromEntries(
    Object.keys(table)
      .sort()
      .map((key) => [key, table[key]]),
  );
}

function sdkTool(args: string[], environment: NodeJS.ProcessEnv): string {
  return execFileSync("/usr/bin/xcrun", ["--sdk", environment.SDKROOT || "macosx", ...args], {
    encoding: "utf8",
    env: environment,
  }).trim();
}

// Use the same SDK selection that Apple's compiler tools receive.
function assertSdkVersion(sdk: string, environment: NodeJS.ProcessEnv): void {
  const actual = sdkTool(["--show-sdk-version"], environment);
  if (actual !== sdk)
    throw new Error(
      `Lens requires macOS SDK ${sdk}; selected SDK is ${actual}. Select the required Xcode with DEVELOPER_DIR or xcode-select.`,
    );
}

export function assertMacosSdk(root: string, environment: NodeJS.ProcessEnv = process.env): void {
  assertSdkVersion(bundleContract(root).sdk, environment);
}

export function macosBuildContract(root: string, environment: NodeJS.ProcessEnv = process.env) {
  const { minimum, sdk } = bundleContract(root);
  assertSdkVersion(sdk, environment);
  const keyInputs = {
    manifest: cacheManifest(root),
    minimum,
    sdk,
    sdkBuild: sdkTool(["--show-sdk-build-version"], environment),
    xcode: execFileSync("/usr/bin/xcodebuild", ["-version"], {
      encoding: "utf8",
      env: environment,
    }).trim(),
    clang: sdkTool(["clang", "--version"], environment).split("\n", 1)[0],
    swift: sdkTool(["swiftc", "--version"], environment).split("\n", 1)[0],
    cargo: execFileSync("cargo", ["-Vv"], { cwd: root, encoding: "utf8", env: environment })
      .trim()
      .split("\n", 1)[0],
    // rust-cache already keys CARGO, RUST, CC, CFLAGS, CXX and CMAKE variables.
    appleBuildEnvironment: {
      CPPFLAGS: environment.CPPFLAGS,
      LDFLAGS: environment.LDFLAGS,
      SWIFTFLAGS: environment.SWIFTFLAGS,
      TOOLCHAINS: environment.TOOLCHAINS,
    },
  };
  const digest = createHash("sha256")
    .update(JSON.stringify(keyInputs, canonicalToml))
    .digest("hex");
  return {
    cacheKey: `${MACOS_DEPENDENCY_CACHE_REVISION}-${digest}`,
    deploymentTarget: minimum,
  };
}

export function publishMacosBuildContract(root: string, environment: NodeJS.ProcessEnv): void {
  const contract = macosBuildContract(root, environment);
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
  const root = fileURLToPath(new URL("..", import.meta.url));
  publishMacosBuildContract(root, process.env);
}
