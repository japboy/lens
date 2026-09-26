import { describe, expect, it } from "vitest";
import {
  VERIFICATION_REQUIREMENTS,
  FRONTEND_TEST_INPUTS,
  classifyChange,
  parseChangedPaths,
  parseRequirementOutputs,
  planChanges,
  requireCiResults,
  validateRequirements,
} from "./ci-plan.ts";
import type { Change } from "./ci-plan.ts";

const body: Change = {
  path: "packages/usecase/src/state.rs",
  before: "pub fn value() -> u8 { 1 }",
  after: "pub fn value() -> u8 { 2 }",
};
const pathChange = (path: string): Change => ({ path, before: "before", after: "after" });
const [frontend, shared, code, app, dmg] = VERIFICATION_REQUIREMENTS;

describe("finite verification requirements with per-input reasons", () => {
  it.each([
    ["apps/desktop/src/pages/about-page.ts", frontend, "frontend-source"],
    ["apps/desktop/tests/fixtures/README.html", frontend, "frontend-source"],
    ...FRONTEND_TEST_INPUTS.map((path) => [path, frontend, "frontend-test"] as const),
    ["LICENSE", code, "native-input"],
    ["NOTICE", code, "native-input"],
    ["apps/desktop/tests/fixtures/workspace-contracts.json", code, "native-input"],
    ["apps/desktop/tests/fixtures/acp-generated-image.json", code, "native-input"],
    ["apps/desktop/src/html-output.ts", code, "native-input"],
    ["apps/desktop/src/application/webview-port.ts", code, "frontend-native-contract"],
    ["apps/desktop/src/types.ts", code, "frontend-native-contract"],
    ["packages/port-platform/src/lib.rs", code, "native-code"],
    ["apps/desktop/src-tauri/src/app_state.rs", code, "native-code"],
    ["apps/desktop/src-tauri/src/native/mod.rs", app, "native-application"],
    ["packages/adapter-platform-macos/native/LensNative.m", app, "native-application"],
    ["apps/desktop/src-tauri/tauri.macos.conf.json", dmg, "control-plane"],
    ["apps/desktop/src-tauri/agent-runtime/pnpm-workspace.yaml", dmg, "control-plane"],
    ["apps/desktop/src/package.json", dmg, "control-plane"],
    ["scripts/new-policy.ts", dmg, "control-plane"],
    ["mise-tasks/check/identity.ts", dmg, "control-plane"],
    ["packages/typescript-config/base.json", dmg, "control-plane"],
    ["Cargo.lock", dmg, "control-plane"],
    ["mise.toml", dmg, "unreviewed-input"],
    ["mise.lock", dmg, "unreviewed-input"],
    ["package.json", dmg, "control-plane"],
    ["apps/desktop/package.json", dmg, "control-plane"],
    ["apps/desktop/tooling/new.test.ts", dmg, "unreviewed-input"],
    ["unknown/path", dmg, "unreviewed-input"],
  ] as const)("classifies %s with explicit ownership", (path, requirements, ruleId) => {
    const result = classifyChange(pathChange(path));
    expect(result).toMatchObject({ path, requirements, ruleId });
    expect(result.reason.length).toBeGreaterThan(0);
  });

  it("admits body-only shared implementation changes", () => {
    expect(planChanges([body]).requirements).toEqual(shared);
    expect(
      planChanges([body, { ...body, path: "packages/domain/src/lens.rs" }, pathChange("README.md")])
        .requirements,
    ).toEqual(shared);
  });

  it("requires native consumers for declarations, constants, additions and deletions", () => {
    for (const change of [
      { ...body, after: "pub fn value() -> u16 { 1 }" },
      { ...body, before: "const N: usize = 1;", after: "const N: usize = 2;" },
      { ...body, before: null },
      { ...body, after: null },
    ])
      expect(classifyChange(change).requirements).toEqual(code);
  });

  it("joins all inputs and preserves native obligations for mixed sets and renames", () => {
    expect(
      planChanges([pathChange(FRONTEND_TEST_INPUTS[0]), pathChange("LICENSE")]).requirements,
    ).toEqual(code);
    expect(
      planChanges([
        { ...pathChange("apps/desktop/tests/fixtures/workspace-contracts.json"), after: null },
        { ...pathChange("apps/desktop/tests/fixtures/ordinary.json"), before: null },
      ]).requirements,
    ).toEqual(code);
    expect(
      planChanges([
        { ...pathChange("packages/adapter-platform-macos/native/old.m"), after: null },
        { ...pathChange("apps/desktop/public/old.svg"), before: null },
      ]).requirements,
    ).toEqual(app);
    expect(planChanges([body, pathChange("unknown")]).requirements).toEqual(dmg);
    expect(planChanges([])).toEqual({ requirements: dmg, reasons: [] });
    const changes = [body, pathChange("LICENSE"), pathChange("unknown")];
    expect(planChanges(changes)).toEqual(planChanges([...changes].reverse()));
  });

  it("never hides a later parser or boundary failure behind complete verification", () => {
    expect(() => planChanges([pathChange("unknown"), { ...body, after: "unknown!();" }])).toThrow(
      /Portable boundary failure|Unreviewed/u,
    );
    expect(() => planChanges([{ ...body, after: "pub fn value() {" }])).toThrow(
      "Incomplete Rust token group",
    );
  });

  it.each(["path", "a\0\0", "a\0a\0", "../a\0", "/a\0", "a//b\0", "a\\b\0", "a\nb\0"])(
    "rejects invalid complete-diff input (%#)",
    (input) => {
      expect(() => parseChangedPaths(input)).toThrow(/Incomplete|Invalid|Duplicate/u);
    },
  );
  it("accepts a complete NUL list and rejects duplicate change objects", () => {
    expect(parseChangedPaths("b file\0a file\0")).toEqual(["a file", "b file"]);
    expect(parseChangedPaths("")).toEqual([]);
    expect(() => planChanges([body, body])).toThrow("Duplicate changed path");
  });
});

describe("canonical requirement and complete four-owner result admission", () => {
  it("admits exactly five records and exact workflow output strings", () => {
    for (const requirements of VERIFICATION_REQUIREMENTS) {
      expect(validateRequirements(requirements)).toEqual(requirements);
      expect(parseRequirementOutputs(String(requirements.sharedRust), requirements.macos)).toEqual(
        requirements,
      );
    }
    for (const value of [
      null,
      {},
      [],
      "full",
      { sharedRust: false, macos: "code" },
      { sharedRust: true, macos: "bundle" },
      { sharedRust: "true", macos: "dmg" },
      { ...dmg, extra: true },
    ])
      expect(() => validateRequirements(value)).toThrow("Invalid verification requirements");
    for (const value of ["", "TRUE", "1", " true", "false\n"])
      expect(() => parseRequirementOutputs(value, "none")).toThrow(/Invalid|Incomplete/u);
    for (const value of ["", "bundle", "none\n", "code "])
      expect(() => parseRequirementOutputs("true", value)).toThrow(/Invalid|Incomplete/u);
    expect(() => parseRequirementOutputs("false", "app")).toThrow(/Invalid|Incomplete/u);
  });

  it("accepts only the required success/authorized-skip tuple across 3,125 cases", () => {
    const outcomes = ["success", "failure", "cancelled", "skipped", ""];
    let cases = 0;
    for (const requirements of VERIFICATION_REQUIREMENTS) {
      const expected = [
        "success",
        "success",
        requirements.sharedRust ? "success" : "skipped",
        requirements.macos === "none" ? "skipped" : "success",
      ];
      expect(() =>
        requireCiResults(requirements, ...(expected as [string, string, string, string])),
      ).not.toThrow();
      cases += 1;
      const invalid = outcomes
        .flatMap((repository) =>
          outcomes.flatMap((frontend) =>
            outcomes.flatMap((sharedRust) =>
              outcomes.map((macos) => [repository, frontend, sharedRust, macos] as const),
            ),
          ),
        )
        .filter((tuple) => tuple.join(":") !== expected.join(":"));
      for (const [repository, frontend, sharedRust, macos] of invalid) {
        expect(() =>
          requireCiResults(requirements, repository, frontend, sharedRust, macos),
        ).toThrow("Incomplete CI result");
        cases += 1;
      }
      expect(() =>
        requireCiResults(requirements, "unknown", "success", "success", "success"),
      ).toThrow(/Invalid|Incomplete/u);
    }
    expect(cases).toBe(3125);
  });
});
