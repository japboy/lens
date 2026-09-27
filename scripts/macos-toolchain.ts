import { execFileSync } from "node:child_process";
import { realpathSync, appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Build SDK and deployment target are independent: new AppKit behavior, old OS support.
export const MACOS_TOOLCHAIN = {
  xcode: "27.0",
  xcodeBuild: "27A266a",
  sdk: "27.0",
  sdkBuild: "26A425",
  minimum: "15.2",
} as const;

export function normalizeVersion(value: string): string {
  if (!/^\d+(?:\.\d+){0,2}$/u.test(value)) throw new Error(`Invalid version: ${value}`);
  return value.split(".").map(Number).concat([0, 0]).slice(0, 3).join(".");
}

export function assertToolchainVersions(xcode: string, sdk: string, sdkBuild: string): void {
  const match = /^Xcode (\S+)\r?\nBuild version (\S+)$/u.exec(xcode.trim());
  if (
    !match ||
    normalizeVersion(match[1]!) !== normalizeVersion(MACOS_TOOLCHAIN.xcode) ||
    match[2] !== MACOS_TOOLCHAIN.xcodeBuild ||
    normalizeVersion(sdk.trim()) !== normalizeVersion(MACOS_TOOLCHAIN.sdk) ||
    sdkBuild.trim() !== MACOS_TOOLCHAIN.sdkBuild
  )
    throw new Error(
      `Lens requires Xcode ${MACOS_TOOLCHAIN.xcode} (${MACOS_TOOLCHAIN.xcodeBuild}), macOS SDK ${MACOS_TOOLCHAIN.sdk} (${MACOS_TOOLCHAIN.sdkBuild}); select it with DEVELOPER_DIR`,
    );
}

export function macosToolchainEnvironment(
  environment: NodeJS.ProcessEnv = process.env,
): NodeJS.ProcessEnv {
  if (process.platform !== "darwin" || process.arch !== "arm64")
    throw new Error("The macOS toolchain requires an Apple-silicon host");
  for (const [name, value] of Object.entries(environment)) {
    if (
      value &&
      /^(?:(?:HOST_|TARGET_)?(?:CC|CXX|OBJC|AR|LD|CFLAGS|CXXFLAGS|OBJCFLAGS|LDFLAGS)(?:_|$)|TOOLCHAINS$|RUSTC$|RUSTFLAGS$|CARGO_ENCODED_RUSTFLAGS$|RUSTC_WRAPPER$|RUSTC_WORKSPACE_WRAPPER$|CARGO_BUILD_RUSTFLAGS$|CARGO_TARGET_.+_(?:RUSTFLAGS|LINKER)$)/u.test(
        name,
      )
    )
      throw new Error(`Unreviewed macOS compiler override: ${name}`);
  }
  const command = (file: string, args: string[], env = environment) =>
    execFileSync(file, args, { encoding: "utf8", env }).trim();
  const developer = realpathSync(
    environment.DEVELOPER_DIR || command("/usr/bin/xcode-select", ["-p"]),
  );
  const selected: NodeJS.ProcessEnv = { ...environment, DEVELOPER_DIR: developer };
  // SDKROOT must not redirect xcrun while discovering the selected Xcode's SDK.
  delete selected.SDKROOT;
  const sdkRoot = realpathSync(
    command("/usr/bin/xcrun", ["--sdk", "macosx", "--show-sdk-path"], selected),
  );
  if (environment.SDKROOT && realpathSync(environment.SDKROOT) !== sdkRoot)
    throw new Error("SDKROOT differs from the selected Xcode macOS SDK");
  assertToolchainVersions(
    command("/usr/bin/xcodebuild", ["-version"], selected),
    command("/usr/bin/xcrun", ["--sdk", "macosx", "--show-sdk-version"], selected),
    command("/usr/bin/xcrun", ["--sdk", "macosx", "--show-sdk-build-version"], selected),
  );
  if (
    environment.MACOSX_DEPLOYMENT_TARGET &&
    normalizeVersion(environment.MACOSX_DEPLOYMENT_TARGET) !==
      normalizeVersion(MACOS_TOOLCHAIN.minimum)
  )
    throw new Error("MACOSX_DEPLOYMENT_TARGET differs from the supported minimum OS");
  return { ...selected, SDKROOT: sdkRoot, MACOSX_DEPLOYMENT_TARGET: MACOS_TOOLCHAIN.minimum };
}

export function assertMachOBuildVersion(output: string, minimum: string): void {
  const commands = output
    .split(/Load command \d+\r?\n/u)
    .filter((command) => /^\s*cmd LC_BUILD_VERSION$/mu.test(command));
  if (commands.length !== 1) throw new Error("Expected one Mach-O LC_BUILD_VERSION");
  const command = commands[0]!;
  const value = (key: string) => new RegExp(`^\\s*${key} (\\S+)\\s*$`, "mu").exec(command)?.[1];
  if (
    value("platform") !== "1" ||
    normalizeVersion(value("sdk") ?? "") !== normalizeVersion(MACOS_TOOLCHAIN.sdk) ||
    normalizeVersion(value("minos") ?? "") !== normalizeVersion(minimum)
  )
    throw new Error("Mach-O platform, SDK or minimum OS differs from the build contract");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const [mode, ...args] = process.argv.slice(2);
  const env = macosToolchainEnvironment();
  if (mode === "run" && args.length) {
    execFileSync(args[0]!, args.slice(1), { env, stdio: "inherit" });
  } else if (mode === "export" && args.length === 0) {
    if (!process.env.GITHUB_ENV) throw new Error("GITHUB_ENV is required for export");
    for (const name of ["DEVELOPER_DIR", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET"])
      appendFileSync(process.env.GITHUB_ENV, `${name}=${env[name]}\n`);
    process.stdout.write(`${JSON.stringify(MACOS_TOOLCHAIN)}\n`);
  } else if (mode === "check" && args.length === 0) {
    process.stdout.write(
      `${JSON.stringify({ ...MACOS_TOOLCHAIN, developer: env.DEVELOPER_DIR, sdkRoot: env.SDKROOT })}\n`,
    );
  } else throw new Error("Expected check, export, or run <command> [arguments]");
}
