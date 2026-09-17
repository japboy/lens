import { execFileSync } from "node:child_process";
import { afterEach, describe, expect, it, vi } from "vitest";
import { assertBuildEnvironment, runVariant, selectVariant } from "./run-workspace-variant.ts";
import { BUILD_VARIANTS, variantArguments } from "./workspace-policy.ts";

vi.mock("node:child_process", () => ({ execFileSync: vi.fn<typeof execFileSync>() }));

const execute = vi.mocked(execFileSync);
const root = "/fixture";

function mockGraph(host: string, version = "2.7.2", tauriFeatures = "", nativePortable = false) {
  execute.mockImplementation((command, args) => {
    if (command === "rustc") return `host: ${host}\nrelease: 1.94.0\n`;
    if (args?.[0] !== "tree") return "";
    const packages = args.flatMap((arg, index) => (arg === "--package" ? [args[index + 1]!] : []));
    return packages
      .map((name) =>
        name === "desktop"
          ? `0desktop v0.3.0 (${root}/apps/desktop/src-tauri)|\n1tauri v2.10.0|${tauriFeatures}\n1tauri-plugin-dialog v${version}|\n`
          : `0${name} v0.3.0 (${root}/packages/${name})|\n${nativePortable ? "1tauri v2.10.0|\n" : ""}`,
      )
      .join("\n");
  });
}

afterEach(() => {
  vi.restoreAllMocks();
  execute.mockReset();
});

describe("explicit compiler variants", () => {
  it.each(["2.7.2", "2.7.3"])(
    "compiles a valid dependency graph at version %s without a saved approval baseline",
    (version) => {
      vi.spyOn(process.stdout, "write").mockReturnValue(true);
      mockGraph("aarch64-apple-darwin", version);
      runVariant("macos-production-check", root);
      expect(execute).toHaveBeenLastCalledWith(
        "cargo",
        variantArguments(selectVariant("macos-production-check")),
        { cwd: root, stdio: "inherit" },
      );
    },
  );

  it("selects only explicitly declared compiler invocations", () => {
    for (const variant of BUILD_VARIANTS) expect(selectVariant(variant.id)).toEqual(variant);
    expect(() => runVariant("implicit-workspace-defaults", root)).toThrow(
      "Unknown workspace variant",
    );
    expect(execute).not.toHaveBeenCalled();
  });

  it("rejects unsupported analysis hosts", () => {
    mockGraph("unknown");
    expect(() => runVariant("macos-production-check", root)).toThrow("graph-analysis host");
    expect(execute).toHaveBeenCalledTimes(1);
  });

  it("requires the declared native host before compiling", () => {
    mockGraph("x86_64-unknown-linux-gnu");
    expect(() => runVariant("macos-production-check", root)).toThrow("require their declared host");
    expect(execute).toHaveBeenCalledTimes(2);
  });

  it("permits the explicitly declared portable Apple cross-check", () => {
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    mockGraph("x86_64-unknown-linux-gnu");
    runVariant("apple-portable-check", root);
    expect(execute).toHaveBeenLastCalledWith(
      "cargo",
      variantArguments(selectVariant("apple-portable-check")),
      { cwd: root, stdio: "inherit" },
    );
  });

  it("rejects test features in the production dependency graph before compiling", () => {
    mockGraph("aarch64-apple-darwin", "2.7.3", "test");
    expect(() => runVariant("macos-production-check", root)).toThrow(
      "Tauri feature isolation failed",
    );
    expect(execute).toHaveBeenCalledTimes(2);
  });

  it("rejects a native shell in the portable dependency graph before compiling", () => {
    mockGraph("aarch64-apple-darwin", "2.7.3", "", true);
    expect(() => runVariant("apple-portable-check", root)).toThrow(
      "portable transitive graph contains native shell",
    );
    expect(execute).toHaveBeenCalledTimes(2);
  });

  it.each([
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS",
  ])("rejects hidden compiler override %s", (name) => {
    expect(() => assertBuildEnvironment({ [name]: "override" })).toThrow(
      "Unreviewed compiler override",
    );
  });

  it("permits cache location and empty override settings", () => {
    expect(() =>
      assertBuildEnvironment({ CARGO_TARGET_DIR: "/tmp/target", RUSTFLAGS: "" }),
    ).not.toThrow();
  });
});
