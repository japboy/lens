import { describe, expect, it } from "vitest";
import { sidecarBuildPlan, tauriOutputArguments } from "./output-sidecar.ts";

describe("output sidecar distribution contract", () => {
  it("uses target-suffixed Tauri inputs and matching Cargo profiles", () => {
    expect(sidecarBuildPlan("aarch64-apple-darwin", "dev")).toMatchObject({
      executable: "lens-output-mcp",
      stagedName: "lens-output-mcp-aarch64-apple-darwin",
      profileDirectory: "debug",
    });
    expect(sidecarBuildPlan("aarch64-apple-darwin", "release").arguments).toContain("release");
    expect(() => sidecarBuildPlan("../../other", "dev")).toThrow("Unsupported");
  });
  it("prepares dev/build/bundle and leaves informational commands alone", () => {
    expect(tauriOutputArguments(["dev"])).toMatchObject({
      profile: "dev",
      arguments: ["dev", "--config", "src-tauri/tauri.output.conf.json"],
    });
    expect(tauriOutputArguments(["build", "--target", "aarch64-apple-darwin"])).toMatchObject({
      profile: "release",
      target: "aarch64-apple-darwin",
    });
    expect(tauriOutputArguments(["build", "--debug"]).profile).toBe("dev");
    expect(tauriOutputArguments(["build", "-d", "-t", "aarch64-apple-darwin"])).toMatchObject({
      profile: "dev",
      target: "aarch64-apple-darwin",
    });
    expect(
      tauriOutputArguments(["dev", "--", "--help", "--target", "application-option"]),
    ).toMatchObject({ profile: "dev" });
    expect(tauriOutputArguments(["info"])).toEqual({ arguments: ["info"] });
    expect(tauriOutputArguments(["build", "--help"])).toEqual({ arguments: ["build", "--help"] });
    expect(() => tauriOutputArguments(["build", "--profile", "custom"])).toThrow("profiles");
  });
});
