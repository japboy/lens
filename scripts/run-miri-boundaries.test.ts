import { describe, expect, it } from "vitest";
import {
  selectedMiriTests,
  validateMiriResult,
  validateMiriSelection,
} from "./run-miri-boundaries.ts";
import { miriExclusionViolations, unsafeExceptionViolations } from "./rust-source-boundaries.ts";

describe("explicit production Miri boundary selection", () => {
  it("requires exact nonempty discovered tests, including both ownership groups", () => {
    const tests = selectedMiriTests();
    expect(tests.some((name) => name.startsWith("presentation::tests::"))).toBe(true);
    expect(tests.some((name) => name.startsWith("accessibility::tests::miri_boundary_"))).toBe(
      true,
    );
    expect(() =>
      validateMiriSelection(tests.map((name) => `${name}: test`).join("\n"), tests),
    ).not.toThrow();
    expect(() => validateMiriSelection("0 tests, 0 benchmarks", tests)).toThrow(/Miri/u);
    expect(() => validateMiriSelection("unreviewed: test", tests)).toThrow(/Miri/u);
    expect(() => validateMiriSelection("", [])).toThrow(/Miri/u);
  });
  it("rejects ignored tests and partial execution even when the harness reports success", () => {
    expect(() =>
      validateMiriResult("test result: ok. 15 passed; 0 failed; 0 ignored;", 15),
    ).not.toThrow();
    expect(() =>
      validateMiriResult("test result: ok. 14 passed; 0 failed; 1 ignored;", 15),
    ).toThrow(/Miri/u);
    expect(() => validateMiriResult("test result: ok. 0 passed; 0 failed; 0 ignored;", 15)).toThrow(
      /Miri/u,
    );
  });
  it("rejects Miri-only branches after nested cfg predicates", () => {
    const owner = "packages/adapter-platform-macos/src/accessibility.rs";
    for (const source of [
      "if cfg!(all(any(test), miri)) { return; }",
      "if cfg![all(any(test), miri)] { return; }",
      "if cfg!{all(any(test), miri)} { return; }",
      "#[cfg(all(any(test), not(miri)))] fn skipped() {}",
      "#![cfg(all(any(test), not(miri)))]",
    ]) {
      expect(miriExclusionViolations(source, owner)).not.toEqual([]);
    }
    expect(
      miriExclusionViolations('if cfg!(all(any(test), target_os = "miri")) {}', owner),
    ).toEqual([]);
  });
  it("requires a declared owner/category and review for Miri-only exclusions", () => {
    const source =
      '#[allow(unsafe_code, reason = "[boundary:ax-ffi] Borrowed inputs remain live")] unsafe {}';
    expect(
      unsafeExceptionViolations(source, "packages/adapter-platform-macos/src/accessibility.rs"),
    ).toEqual([]);
    expect(unsafeExceptionViolations(source, "packages/domain/src/lib.rs")).not.toEqual([]);
    expect(unsafeExceptionViolations(source.replace("ax-ffi", "unknown"))).not.toEqual([]);
    const owner = "packages/adapter-platform-macos/src/accessibility.rs";
    expect(miriExclusionViolations("#[cfg(not(miri))] fn skipped() {}", owner)).not.toEqual([]);
    expect(
      miriExclusionViolations('#[cfg_attr(miri, ignore = "unsupported")] fn skipped() {}', owner),
    ).not.toEqual([]);
    expect(miriExclusionViolations("#[ignore] fn skipped() {}", owner)).not.toEqual([]);
    expect(miriExclusionViolations("if cfg!(miri) { return; }", owner)).not.toEqual([]);
  });
});
