import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { sourceDigest, sourceInputs } from "../apps/desktop/tooling/prerender/source.ts";
import { verifyGeneration } from "../apps/desktop/tooling/prerender/verify.ts";
import { bundleContract } from "./release/bundle.ts";
import { runVariant } from "./run-workspace-variant.ts";
import { buildMacosBundle } from "./macos-bundle-build.ts";

vi.mock("../apps/desktop/tooling/prerender/source.ts", () => ({
  sourceDigest: vi.fn<typeof sourceDigest>(),
  sourceInputs: vi.fn<typeof sourceInputs>(),
}));
vi.mock("../apps/desktop/tooling/prerender/verify.ts", () => ({
  verifyGeneration: vi.fn<typeof verifyGeneration>(),
}));
vi.mock("./release/bundle.ts", () => ({ bundleContract: vi.fn<typeof bundleContract>() }));
vi.mock("./run-workspace-variant.ts", () => ({ runVariant: vi.fn<typeof runVariant>() }));

beforeEach(() => {
  vi.stubGlobal("process", { ...process, platform: "darwin", arch: "arm64", env: {} });
  vi.mocked(bundleContract).mockReturnValue({
    minimum: "15.2",
    version: "1.0.0",
    product: "Lens",
    identifier: "com.github.japboy.lens",
    executable: "lens",
  });
  vi.mocked(sourceInputs).mockReturnValue(new Map());
  vi.mocked(sourceDigest).mockReturnValue("current-source-digest");
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetAllMocks();
});

describe("shared packaged macOS compilation", () => {
  it("validates current frontend generation and builds the exact packaged variant at the minimum OS", () => {
    vi.mocked(runVariant).mockImplementation(() => {
      expect(process.env.MACOSX_DEPLOYMENT_TARGET).toBe("15.2");
      expect(verifyGeneration).toHaveBeenCalledWith(
        "/source/apps/desktop/.build/webview",
        "current-source-digest",
      );
    });
    buildMacosBundle("/source");
    expect(runVariant).toHaveBeenCalledExactlyOnceWith("macos-bundle-build", "/source");
    expect(process.env.MACOSX_DEPLOYMENT_TARGET).toBeUndefined();
  });

  it("rejects stale or missing frontend generation before invoking Cargo", () => {
    vi.mocked(verifyGeneration).mockImplementation(() => {
      throw new Error("stale generation");
    });
    expect(() => buildMacosBundle("/source")).toThrow("stale generation");
    expect(runVariant).not.toHaveBeenCalled();
  });

  it("rejects a conflicting deployment environment before invoking Cargo", () => {
    process.env.MACOSX_DEPLOYMENT_TARGET = "14.0";
    expect(() => buildMacosBundle("/source")).toThrow("deployment environment differs");
    expect(runVariant).not.toHaveBeenCalled();
    expect(process.env.MACOSX_DEPLOYMENT_TARGET).toBe("14.0");
  });

  it("restores the inherited environment when compilation fails", () => {
    process.env.MACOSX_DEPLOYMENT_TARGET = "15.2";
    vi.mocked(runVariant).mockImplementation(() => {
      throw new Error("compile failed");
    });
    expect(() => buildMacosBundle("/source")).toThrow("compile failed");
    expect(process.env.MACOSX_DEPLOYMENT_TARGET).toBe("15.2");
  });

  it("rejects unsupported hosts before consuming assets", () => {
    vi.stubGlobal("process", { ...process, platform: "linux" });
    expect(() => buildMacosBundle("/source")).toThrow("Apple-silicon host");
    expect(verifyGeneration).not.toHaveBeenCalled();
    expect(runVariant).not.toHaveBeenCalled();
  });
});
