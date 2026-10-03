import { describe, expect, it } from "vitest";
import { BUILD_VARIANTS, MEMBERS, variantArguments } from "./workspace-policy.ts";

describe("presentation and build workspace ownership", () => {
  it("declares shared UI and Node generation with independent application consumers", () => {
    const member = (name: string) =>
      MEMBERS.find((entry) => entry.ecosystem === "pnpm" && entry.name === name)!;
    expect(member("ui")).toMatchObject({
      directory: "packages/ui",
      role: "presentation",
      implementation: "webview",
    });
    expect(member("adapter-lit-prerenderer")).toMatchObject({
      directory: "packages/adapter-lit-prerenderer",
      role: "adapter",
      capability: "repository",
      implementation: "tooling",
    });
    for (const name of ["desktop", "ui-preview"]) {
      expect(member(name).dependencies.normal).toContain("ui");
    }
    expect(member("desktop").dependencies.dev).toContain("adapter-lit-prerenderer");
    for (const name of ["ui", "adapter-lit-prerenderer", "ui-preview"]) {
      expect(Object.values(member(name).dependencies).flat()).not.toContain("desktop");
    }
  });
});

describe("explicit production and test variants", () => {
  it("uses finite exact target/package/feature/profile commands without workspace-wide Linux selection", () => {
    expect(new Set(BUILD_VARIANTS.map((variant) => variant.id)).size).toBe(BUILD_VARIANTS.length);
    for (const variant of BUILD_VARIANTS) {
      const args = variantArguments(variant);
      expect(args).toContain("--locked");
      expect(args).not.toContain("--workspace");
      expect(args).not.toContain("--all-targets");
      expect(variant.profile === "test").toBe(variant.operation === "test");
    }
    for (const variant of BUILD_VARIANTS.filter(
      (entry) => entry.target === "x86_64-unknown-linux-gnu",
    )) {
      expect(variant.packages).toContain("desktop");
      expect(variant.packages).not.toContain("adapter-platform-macos");
    }
    expect(BUILD_VARIANTS.some((variant) => variant.profile === "release")).toBe(true);
  });
});
