import { describe, expect, it } from "vitest";
import { BUILD_VARIANTS, variantArguments } from "./workspace-policy.ts";

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
