import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { runVariant } from "./run-workspace-variant.ts";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
export function verifyNativeBundle(root = ROOT): void {
  if (process.platform !== "darwin" || process.arch !== "arm64")
    throw new Error("Bundle verification requires the admitted Apple-silicon host");
  const application = join(root, "apps/desktop");
  const base = JSON.parse(readFileSync(join(application, "src-tauri/tauri.conf.json"), "utf8"));
  const mac = JSON.parse(
    readFileSync(join(application, "src-tauri/tauri.macos.conf.json"), "utf8"),
  );
  if (!existsSync(join(application, "dist/index.html")))
    throw new Error("Prebuilt frontend artifact is required");
  const minimum = mac.bundle.macOS.minimumSystemVersion;
  if (typeof minimum !== "string" || !/^\d+\.\d+(?:\.\d+)?$/u.test(minimum))
    throw new Error("Explicit minimum macOS version is required");
  const previous = process.env.MACOSX_DEPLOYMENT_TARGET;
  try {
    process.env.MACOSX_DEPLOYMENT_TARGET = minimum;
    runVariant("macos-bundle-build", root);
    // bundle does not run beforeBuildCommand: Linux owns the supplied frontend assets.
    execFileSync(
      "pnpm",
      [
        "exec",
        "tauri",
        "bundle",
        "--target",
        "aarch64-apple-darwin",
        "--features",
        "tauri/custom-protocol",
        "--bundles",
        "app",
        "--ci",
      ],
      { cwd: application, stdio: "inherit" },
    );
  } finally {
    if (previous === undefined) delete process.env.MACOSX_DEPLOYMENT_TARGET;
    else process.env.MACOSX_DEPLOYMENT_TARGET = previous;
  }
  const bundle = join(
    root,
    "target/aarch64-apple-darwin/release/bundle/macos",
    `${base.productName}.app`,
  );
  const info = join(bundle, "Contents/Info.plist");
  const field = (key: string) =>
    execFileSync("plutil", ["-extract", key, "raw", "-o", "-", info], { encoding: "utf8" }).trim();
  if (
    field("CFBundleIdentifier") !== base.identifier ||
    field("CFBundleExecutable") !== "lens" ||
    field("CFBundleName") !== base.productName ||
    field("LSMinimumSystemVersion") !== minimum
  )
    throw new Error("Native bundle identity or minimum OS changed");
  const binary = join(bundle, "Contents/MacOS/lens");
  if (execFileSync("lipo", ["-archs", binary], { encoding: "utf8" }).trim() !== "arm64")
    throw new Error("Unexpected bundle architecture");
  execFileSync("codesign", ["--verify", "--deep", "--strict", bundle], { stdio: "inherit" });
  process.stdout.write(
    `${JSON.stringify({ bundle, identifier: base.identifier, executable: "lens", architecture: "arm64", minimumMacOS: minimum, signingIdentity: mac.bundle.macOS.signingIdentity, status: "passed", notarization: "not-checked" })}\n`,
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length !== 2) throw new Error("No implicit bundle override mode is provided");
  verifyNativeBundle();
}
