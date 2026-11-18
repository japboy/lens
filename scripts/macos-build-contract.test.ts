import { execFileSync } from "node:child_process";
import {
  appendFileSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  assertMacosSdk,
  MACOS_DEPENDENCY_CACHE_REVISION,
  macosBuildContract,
  publishMacosBuildContract,
} from "./macos-build-contract.ts";
import { VERSION_FILES } from "./release/version.ts";

vi.mock("node:child_process", () => ({ execFileSync: vi.fn<typeof execFileSync>() }));

const root = fileURLToPath(new URL("..", import.meta.url));
const macosConfig = "apps/desktop/src-tauri/tauri.macos.conf.json";
const directories: string[] = [];
const toolOutputs = {
  "/usr/bin/xcrun --sdk macosx --show-sdk-version": "27.0",
  "/usr/bin/xcrun --sdk macosx --show-sdk-build-version": "26A123",
  "/usr/bin/xcodebuild -version": "Xcode 27.0\nBuild version 27A123",
  "/usr/bin/xcrun --sdk macosx clang --version": "Apple clang version 21.0.0 (clang-2100.1)",
  "/usr/bin/xcrun --sdk macosx swiftc --version": "Apple Swift version 6.4 (swiftlang-6.4.1)",
  "cargo -Vv": "cargo 1.100.0 (abc 2026-11-01)",
};

let commands: Map<string, string>;
beforeEach(() => {
  commands = new Map(Object.entries(toolOutputs));
  vi.mocked(execFileSync).mockImplementation((command, args = []) => {
    const key = [command, ...args].join(" ");
    const output = commands.get(key);
    if (output === undefined) throw new Error(`Unexpected tool invocation: ${key}`);
    return output;
  });
});

function fixture() {
  const directory = mkdtempSync(join(tmpdir(), "lens-macos-contract-"));
  directories.push(directory);
  for (const path of new Set([...VERSION_FILES, macosConfig, "pnpm-lock.yaml", "mise.toml"])) {
    mkdirSync(dirname(join(directory, path)), { recursive: true });
    copyFileSync(join(root, path), join(directory, path));
  }
  return directory;
}

afterEach(() => {
  for (const directory of directories.splice(0))
    rmSync(directory, { recursive: true, force: true });
  vi.restoreAllMocks();
});

describe("macOS dependency cache build contract", () => {
  it("reuses dependency artifacts across release version and source-tooling changes", () => {
    const directory = fixture();
    const baseline = macosBuildContract(directory, {});
    expect(baseline.deploymentTarget).toBe("15.2");
    expect(baseline.cacheKey).toMatch(
      new RegExp(`^${MACOS_DEPENDENCY_CACHE_REVISION}-[a-f0-9]{64}$`),
    );

    const version = JSON.parse(readFileSync(join(directory, "package.json"), "utf8")).version;
    for (const path of VERSION_FILES) {
      const file = join(directory, path);
      writeFileSync(file, readFileSync(file, "utf8").replaceAll(version, "999.0.0"));
    }
    appendFileSync(join(directory, "pnpm-lock.yaml"), "# frontend dependency lock change\n");
    appendFileSync(join(directory, "mise.toml"), "# tooling change\n");
    const tauriConfig = join(directory, "apps/desktop/src-tauri/tauri.conf.json");
    const tauri = JSON.parse(readFileSync(tauriConfig, "utf8"));
    tauri.bundle.category = "Utility";
    writeFileSync(tauriConfig, JSON.stringify(tauri));
    expect(macosBuildContract(directory, {})).toEqual(baseline);
  });

  it("ignores virtual-root manifest formatting and table key order", () => {
    const directory = fixture();
    const cargo = join(directory, "Cargo.toml");
    const baseline = macosBuildContract(directory, {});
    const manifest = readFileSync(cargo, "utf8");
    writeFileSync(
      cargo,
      "# compatibility does not depend on comments\n" +
        manifest.replace(/(^codegen-units = .*\n)(lto = .*\n)/mu, "$2$1"),
    );
    expect(macosBuildContract(directory, {})).toEqual(baseline);
  });

  it.each([
    ["lto = true", "lto = false"],
    ['resolver = "2"', 'resolver = "3"'],
    ['edition = "2021"', 'edition = "2024"'],
    ['features = ["tracing"]', 'features = ["stdio"]'],
  ])("invalidates on a virtual-root setting change from %s to %s", (before, after) => {
    const directory = fixture();
    const baseline = macosBuildContract(directory, {}).cacheKey;
    const cargo = join(directory, "Cargo.toml");
    const manifest = readFileSync(cargo, "utf8");
    expect(manifest).toContain(before);
    writeFileSync(cargo, manifest.replace(before, after));
    expect(macosBuildContract(directory, {}).cacheKey).not.toBe(baseline);
  });

  it.each([
    "/usr/bin/xcrun --sdk macosx --show-sdk-build-version",
    "/usr/bin/xcodebuild -version",
    "/usr/bin/xcrun --sdk macosx clang --version",
    "/usr/bin/xcrun --sdk macosx swiftc --version",
    "cargo -Vv",
  ])("invalidates when the selected tool identity from %s changes", (tool) => {
    const directory = fixture();
    const baseline = macosBuildContract(directory, {}).cacheKey;
    commands.set(tool, "Different compiler or SDK build");
    expect(macosBuildContract(directory, {}).cacheKey).not.toBe(baseline);
  });

  it("ignores tool installation paths and host details outside the tool version line", () => {
    const directory = fixture();
    const baseline = macosBuildContract(directory, {});
    commands.set(
      "/usr/bin/xcrun --sdk macosx clang --version",
      toolOutputs["/usr/bin/xcrun --sdk macosx clang --version"] + "\nInstalledDir: /alt/Xcode",
    );
    commands.set("cargo -Vv", toolOutputs["cargo -Vv"] + "\nos: Mac OS");
    expect(macosBuildContract(directory, {})).toEqual(baseline);
  });

  it.each(["CPPFLAGS", "LDFLAGS", "SWIFTFLAGS", "TOOLCHAINS"])(
    "invalidates when %s changes",
    (flag) => {
      const directory = fixture();
      expect(macosBuildContract(directory, { [flag]: "changed" }).cacheKey).not.toBe(
        macosBuildContract(directory, {}).cacheKey,
      );
    },
  );

  it("exports the validated deployment environment before cache restoration", () => {
    const directory = fixture();
    const output = join(directory, "output");
    const environment = join(directory, "environment");
    writeFileSync(output, "existing_output=preserved\n");
    writeFileSync(environment, "EXISTING=preserved\n");
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    publishMacosBuildContract(directory, { GITHUB_OUTPUT: output, GITHUB_ENV: environment });
    const { cacheKey } = macosBuildContract(directory, {});
    expect(readFileSync(output, "utf8")).toBe(
      `existing_output=preserved\ncache_key=${cacheKey}\ndeployment_target=15.2\n`,
    );
    expect(readFileSync(environment, "utf8")).toBe(
      "EXISTING=preserved\nMACOSX_DEPLOYMENT_TARGET=15.2\n",
    );
  });

  it("rejects an unreviewed deployment or SDK before publishing the key", () => {
    const directory = fixture();
    const output = join(directory, "output");
    writeFileSync(output, "");
    const mac = join(directory, macosConfig);
    const original = readFileSync(mac, "utf8");
    writeFileSync(mac, original.replace("15.2", "15.3"));
    expect(() => publishMacosBuildContract(directory, { GITHUB_OUTPUT: output })).toThrow(
      "Unreviewed distribution",
    );
    writeFileSync(mac, original);
    commands.set("/usr/bin/xcrun --sdk macosx --show-sdk-version", "27.1");
    expect(() => publishMacosBuildContract(directory, { GITHUB_OUTPUT: output })).toThrow(
      "Lens requires macOS SDK 27.0",
    );
    expect(readFileSync(output, "utf8")).toBe("");
  });
});

describe("effective macOS SDK", () => {
  it("checks the SDK selected by Apple's tools without changing the environment", () => {
    const environment = { DEVELOPER_DIR: "/Applications/Xcode_27.app/Contents/Developer" };
    vi.mocked(execFileSync).mockReturnValue("27.0\n");
    expect(() => assertMacosSdk(root, environment)).not.toThrow();
    expect(execFileSync).toHaveBeenCalledWith(
      "/usr/bin/xcrun",
      ["--sdk", "macosx", "--show-sdk-version"],
      { encoding: "utf8", env: environment },
    );
    expect(environment).toEqual({ DEVELOPER_DIR: "/Applications/Xcode_27.app/Contents/Developer" });
  });
  it("checks an explicit SDKROOT and rejects an older SDK despite a new Xcode selection", () => {
    const environment = {
      DEVELOPER_DIR: "/Applications/Xcode_27.app/Contents/Developer",
      SDKROOT: "/old/MacOSX.sdk",
    };
    vi.mocked(execFileSync).mockReturnValue("15.5\n");
    expect(() => assertMacosSdk(root, environment)).toThrow(
      "Lens requires macOS SDK 27.0; selected SDK is 15.5",
    );
    expect(execFileSync).toHaveBeenCalledWith(
      "/usr/bin/xcrun",
      ["--sdk", environment.SDKROOT, "--show-sdk-version"],
      { encoding: "utf8", env: environment },
    );
  });
});
