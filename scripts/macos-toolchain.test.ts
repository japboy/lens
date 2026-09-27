import { execFileSync } from "node:child_process";
import { realpathSync, appendFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  assertMachOBuildVersion,
  assertToolchainVersions,
  macosToolchainEnvironment,
  normalizeVersion,
} from "./macos-toolchain.ts";

vi.mock("node:child_process", () => ({ execFileSync: vi.fn<typeof execFileSync>() }));
vi.mock("node:fs", () => ({
  realpathSync: vi.fn<typeof realpathSync>(),
  appendFileSync: vi.fn<typeof appendFileSync>(),
}));

beforeEach(() => {
  vi.stubGlobal("process", { ...process, platform: "darwin", arch: "arm64" });
  vi.mocked(realpathSync).mockImplementation((path) => String(path));
  vi.mocked(execFileSync).mockImplementation(((file: string, args: string[]) => {
    if (file === "/usr/bin/xcode-select") return "/selected/Xcode.app/Contents/Developer";
    if (file === "/usr/bin/xcodebuild") return "Xcode 27.0\nBuild version 27A266a\n";
    return (
      {
        "--show-sdk-path": "/selected/sdk",
        "--show-sdk-version": "27.0",
        "--show-sdk-build-version": "26A425",
      } as Record<string, string>
    )[args[2]!]!;
  }) as typeof execFileSync);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetAllMocks();
});

describe("macOS toolchain admission", () => {
  it("normalizes equivalent Apple version spellings", () => {
    expect(normalizeVersion("27")).toBe("27.0.0");
    expect(normalizeVersion("27.0.0")).toBe("27.0.0");
    expect(() =>
      assertToolchainVersions("Xcode 27\nBuild version 27A266a", "27.0.0", "26A425"),
    ).not.toThrow();
    expect(() => normalizeVersion("27 beta")).toThrow("Invalid version");
  });
  it.each([
    ["Xcode 16.4\nBuild version 16F6", "15.5", "24F74"],
    ["Xcode 27.0\nBuild version beta", "27.0", "26A425"],
    ["Xcode 27.0\nBuild version 27A266a", "27.1", "26A425"],
    ["Xcode 27.0\nBuild version 27A266a", "27.0", "other"],
  ])("rejects mismatched compiler or SDK identity", (xcode, sdk, build) => {
    expect(() => assertToolchainVersions(xcode, sdk, build)).toThrow("Lens requires Xcode");
  });
  it("pins child processes without changing caller or global Xcode selection", () => {
    const input = { KEEP: "value", MACOSX_DEPLOYMENT_TARGET: "15.2.0" };
    const output = macosToolchainEnvironment(input);
    expect(output).toEqual({
      KEEP: "value",
      DEVELOPER_DIR: "/selected/Xcode.app/Contents/Developer",
      SDKROOT: "/selected/sdk",
      MACOSX_DEPLOYMENT_TARGET: "15.2",
    });
    expect(input).toEqual({ KEEP: "value", MACOSX_DEPLOYMENT_TARGET: "15.2.0" });
    for (const [file, , options] of vi.mocked(execFileSync).mock.calls) {
      if (file === "/usr/bin/xcode-select") continue;
      expect(options).toMatchObject({
        env: { DEVELOPER_DIR: "/selected/Xcode.app/Contents/Developer" },
      });
      expect((options as { env: NodeJS.ProcessEnv }).env.SDKROOT).toBeUndefined();
    }
    expect(vi.mocked(execFileSync).mock.calls.some(([, args]) => args?.includes("--switch"))).toBe(
      false,
    );
  });
  it("accepts matching explicit paths and rejects an unrelated SDK", () => {
    expect(
      macosToolchainEnvironment({
        DEVELOPER_DIR: "/selected/Xcode.app/Contents/Developer",
        SDKROOT: "/selected/sdk",
      }).SDKROOT,
    ).toBe("/selected/sdk");
    expect(() => macosToolchainEnvironment({ SDKROOT: "/old/sdk" })).toThrow("SDKROOT differs");
  });
  it("rejects a changed minimum OS and unsupported hosts", () => {
    expect(() => macosToolchainEnvironment({ MACOSX_DEPLOYMENT_TARGET: "26" })).toThrow(
      "supported minimum",
    );
    vi.stubGlobal("process", { ...process, platform: "linux" });
    expect(() => macosToolchainEnvironment({})).toThrow("Apple-silicon host");
  });
  it.each([
    "CC",
    "CC_aarch64_apple_darwin",
    "TARGET_CC",
    "TOOLCHAINS",
    "RUSTC",
    "CFLAGS",
    "CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER",
    "RUSTFLAGS",
  ])("rejects compiler redirection through %s", (key) => {
    expect(() => macosToolchainEnvironment({ [key]: "/old/toolchain" })).toThrow(
      "Unreviewed macOS compiler override",
    );
  });
});

const valid =
  "Load command 1\n cmd LC_BUILD_VERSION\n platform 1\n minos 15.2\n sdk 27.0\nLoad command 2\n cmd LC_UUID\n";
describe("packaged Mach-O admission", () => {
  it("accepts the actual new SDK with the independently retained minimum OS", () => {
    expect(() => assertMachOBuildVersion(valid, "15.2")).not.toThrow();
  });
  it.each([
    valid.replace("27.0", "15.5"),
    valid.replace("minos 15.2", "minos 27.0"),
    valid.replace("platform 1", "platform 2"),
    valid + valid,
    "cmd LC_VERSION_MIN_MACOSX",
  ])("rejects incompatible or ambiguous linked metadata", (output) => {
    expect(() => assertMachOBuildVersion(output, "15.2")).toThrow(/Mach-O/u);
  });
});
