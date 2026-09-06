import { describe, expect, it } from "vitest";
import {
  CI_PLANS,
  classifyChange,
  parseChangedPaths,
  planChanges,
  requireCiResults,
} from "./ci-plan.ts";
import type { Change } from "./ci-plan.ts";

const body: Change = {
  path: "packages/usecase/src/state.rs",
  before: "pub fn value() -> u8 { 1 }",
  after: "pub fn value() -> u8 { 2 }",
};
const pathChange = (path: string): Change => ({ path, before: "before", after: "after" });

describe("finite Linux-first change plans", () => {
  it.each([
    ["apps/desktop/src/pages/about-page.ts", "frontend-only"],
    ["apps/desktop/src/styles.css", "frontend-only"],
    ["apps/desktop/tests/fixtures/README.html", "frontend-only"],
    ["apps/desktop/src/application/webview-port.ts", "native-code"],
    ["apps/desktop/src/types.ts", "native-code"],
    ["apps/desktop/src/agent-prompt-template.ts", "native-code"],
    ["packages/port-platform/src/lib.rs", "native-code"],
    ["apps/desktop/src-tauri/src/app_state.rs", "native-code"],
    ["apps/desktop/src-tauri/src/native/mod.rs", "native-bundle"],
    ["packages/adapter-platform-macos/native/LensNative.m", "native-bundle"],
    ["apps/desktop/src-tauri/tauri.macos.conf.json", "full"],
    ["apps/desktop/src-tauri/agent-runtime/claude/pnpm-lock.yaml", "full"],
    ["apps/desktop/src/package.json", "full"],
    ["scripts/new-policy.ts", "full"],
    ["mise-tasks/check/identity.ts", "full"],
    ["packages/typescript-config/base.json", "full"],
    ["Cargo.lock", "full"],
    ["mise.toml", "full"],
    ["unknown/path", "full"],
  ])("classifies %s as %s", (path, expected) => {
    expect(classifyChange(pathChange(path))).toBe(expected);
  });

  it("admits body-only changes to actual shared implementation owners", () => {
    expect(planChanges([body])).toBe("portable-rust");
    expect(
      planChanges([
        body,
        { ...body, path: "packages/domain/src/lens.rs" },
        pathChange("README.md"),
      ]),
    ).toBe("portable-rust");
  });

  it("requires native verification for interfaces, constants, added and deleted Rust files", () => {
    for (const change of [
      { ...body, after: "pub fn value() -> u16 { 1 }" },
      { ...body, before: "const N: usize = 1;", after: "const N: usize = 2;" },
      { ...body, before: null },
      { ...body, after: null },
    ])
      expect(classifyChange(change)).toBe("native-code");
  });

  it("keeps native obligations for mixed sets and cross-boundary renames", () => {
    expect(planChanges([body, pathChange("packages/port-platform/src/lib.rs")])).toBe(
      "native-code",
    );
    expect(
      planChanges([
        { ...pathChange("packages/adapter-platform-macos/native/old.m"), after: null },
        { ...pathChange("apps/desktop/public/old.svg"), before: null },
      ]),
    ).toBe("native-bundle");
    expect(planChanges([body, pathChange("unknown")])).toBe("full");
    expect(planChanges([])).toBe("full");
  });

  it("does not convert a later parser or boundary failure into a native fallback success", () => {
    expect(() => planChanges([pathChange("unknown"), { ...body, after: "unknown!();" }])).toThrow(
      /Portable boundary failure|Unreviewed/u,
    );
    expect(() => planChanges([{ ...body, after: "pub fn value() {" }])).toThrow(
      "Incomplete Rust token group",
    );
  });

  it.each(["path", "a\0\0", "a\0a\0", "../a\0", "/a\0", "a//b\0", "a\\b\0", "a\nb\0"])(
    "rejects incomplete or invalid diff input (%#)",
    (input) => {
      expect(() => parseChangedPaths(input)).toThrow(/Incomplete|Invalid|Duplicate/u);
    },
  );

  it("accepts a complete NUL list, including spaces, and treats an empty diff explicitly", () => {
    expect(parseChangedPaths("b file\0a file\0")).toEqual(["a file", "b file"]);
    expect(parseChangedPaths("")).toEqual([]);
    expect(() => planChanges([body, body])).toThrow("Duplicate changed path");
  });
});

describe("complete required-check results", () => {
  it("accepts exactly the declared success/authorized-skip combinations", () => {
    for (const [plan, required] of Object.entries(CI_PLANS)) {
      expect(() =>
        requireCiResults(
          plan,
          "success",
          required.common ? "success" : "skipped",
          required.native === "none" ? "skipped" : "success",
        ),
      ).not.toThrow();
    }
  });

  it("rejects every other result tuple, including missing output and unexpected skips", () => {
    const outcomes = ["success", "failure", "cancelled", "skipped", "", "unknown"];
    for (const [plan, required] of Object.entries(CI_PLANS)) {
      const success = `success:${required.common ? "success" : "skipped"}:${required.native === "none" ? "skipped" : "success"}`;
      const invalid = outcomes
        .flatMap((portable) =>
          outcomes.flatMap((common) =>
            outcomes.map((native) => [portable, common, native] as const),
          ),
        )
        .filter((tuple) => tuple.join(":") !== success);
      for (const [portable, common, native] of invalid)
        expect(() => requireCiResults(plan, portable, common, native)).toThrow(
          "Incomplete CI result",
        );
    }
    expect(() => requireCiResults("", "success", "success", "success")).toThrow("Unknown CI plan");
    expect(() => requireCiResults("toString", "success", "success", "success")).toThrow(
      "Unknown CI plan",
    );
  });
});
