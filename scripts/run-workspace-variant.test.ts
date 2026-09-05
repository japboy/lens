import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  assertBuildEnvironment,
  selectVariant,
  validateVariantAdmission,
} from "./run-workspace-variant.ts";
import { BUILD_VARIANTS, variantArguments } from "./workspace-policy.ts";
import { graphArguments } from "../mise-tasks/inspect/features.ts";

const admission = JSON.parse(
  readFileSync(new URL("./workspace-variants.json", import.meta.url), "utf8"),
);
function report() {
  const host = "aarch64-apple-darwin";
  return {
    version: 2,
    applicationVersion: "0.1.0",
    host,
    rustc: `host: ${host}\nrelease: ${admission.rustcRelease}\n`,
    variants: BUILD_VARIANTS.map((variant) => ({
      variant: variant.id,
      profile: variant.profile,
      args: graphArguments(variant),
      digest: "f".repeat(64),
      admissionDigest: admission.hosts[host][variant.id].graphDigest,
      graph: { nodes: [], edges: [], roots: [] },
    })),
  };
}

describe("explicit compiler variant admission", () => {
  it("admits exact host/toolchain/graph/invocations, including a single selected variant", () => {
    expect(() => validateVariantAdmission(report(), admission)).not.toThrow();
    const selected = report();
    selected.variants = selected.variants.slice(0, 1);
    expect(() => validateVariantAdmission(selected, admission)).not.toThrow();
    for (const variant of BUILD_VARIANTS) {
      expect(selectVariant(variant.id)).toEqual(variant);
      expect(admission.hosts[selected.host][variant.id].arguments).toEqual(
        variantArguments(variant),
      );
    }
  });

  it.each([
    (value: ReturnType<typeof report>) => {
      value.host = "unknown";
    },
    (value: ReturnType<typeof report>) => {
      value.rustc = "release: 1.0.0";
    },
    (value: ReturnType<typeof report>) => {
      value.variants = [];
    },
    (value: ReturnType<typeof report>) => {
      value.variants.push(value.variants[0]!);
    },
    (value: ReturnType<typeof report>) => {
      value.variants[0]!.variant = "unknown";
    },
    (value: ReturnType<typeof report>) => {
      value.variants[0]!.profile = "release";
    },
    (value: ReturnType<typeof report>) => {
      value.variants[0]!.admissionDigest = "0".repeat(64);
    },
  ])("rejects incomplete or unreviewed compiler evidence (%#)", (mutate) => {
    const value = report();
    mutate(value);
    expect(() => validateVariantAdmission(value, admission)).toThrow(
      /Unreviewed|Incomplete|Empty/u,
    );
  });

  it("rejects policy drift before a compiler invocation", () => {
    const changed = structuredClone(admission);
    changed.hosts[report().host][BUILD_VARIANTS[0]!.id].arguments.push("--all-features");
    expect(() => validateVariantAdmission(report(), changed)).toThrow("Unreviewed package");
    delete changed.hosts[report().host][BUILD_VARIANTS[0]!.id];
    expect(() => validateVariantAdmission(report(), changed)).toThrow("Incomplete host");
    expect(() => selectVariant("implicit-workspace-defaults")).toThrow("Unknown workspace variant");
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
