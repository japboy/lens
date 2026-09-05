import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  commonShellConditionalViolations,
  nativeCompositionViolations,
  rustDeclarationSurface,
} from "./rust-source-surface.ts";

describe("conservative Rust declaration comparison", () => {
  it("discards ordinary implementation bodies, comments and test-only items", () => {
    const before = `#![forbid(unsafe_code)]
      pub struct Value { count: usize }
      impl Value { pub fn count(&self) -> usize { self.count } }
      #[cfg(test)] mod tests { #[test] fn first() { assert!(true); } }`;
    const after = before
      .replace("{ self.count }", '{ let text = r#"{ }"#; 2 }')
      .replace("first()", "second()");
    expect(rustDeclarationSurface(after)).toBe(rustDeclarationSurface(before));
    expect(rustDeclarationSurface(`${before}\n// comment`)).toBe(rustDeclarationSurface(before));
  });

  it.each([
    ["pub fn value() -> u8 { 1 }", "pub fn value() -> u16 { 1 }"],
    ["const N: usize = 1;", "const N: usize = 2;"],
    ["pub const fn size() -> usize { 1 }", "pub const fn size() -> usize { 2 }"],
    ["struct Value { a: u8 }", "struct Value { a: u16 }"],
    ["enum Value { A, B }", "enum Value { A, B, C }"],
    ["use foo::A;", "use bar::A;"],
    ['#[serde(rename = "a")] struct Value;', '#[serde(rename = "b")] struct Value;'],
    ["impl A { fn x() {} }", "impl B { fn x() {} }"],
    ["trait A { fn x(&self); }", "trait A { fn x(&mut self); }"],
    ["type A = [u8; 1];", "type A = [u8; 2];"],
  ])("retains consumer-visible or compile-time changes: %s", (before, after) => {
    expect(rustDeclarationSurface(after)).not.toBe(rustDeclarationSurface(before));
  });

  it.each([
    "fn broken() {",
    "struct Broken { a: (u8] }",
    "#[test]",
    "unknown!();",
    "extern crate other;",
    "fn f<const N: usize = { 1 }>() {}",
  ])("rejects malformed or unsupported item syntax: %s", (source) => {
    expect(() => rustDeclarationSurface(source)).toThrow(
      /Incomplete|Mismatched|Attribute|Unreviewed/u,
    );
  });

  it("parses all actual portable source files", () => {
    const visit = (directory: string): string[] =>
      readdirSync(directory, { withFileTypes: true }).flatMap((entry) =>
        entry.isDirectory()
          ? visit(join(directory, entry.name))
          : entry.name.endsWith(".rs")
            ? [join(directory, entry.name)]
            : [],
      );
    const paths = ["domain", "port-platform", "usecase"].flatMap((name) =>
      visit(`packages/${name}/src`),
    );
    expect(paths.length).toBeGreaterThan(10);
    for (const path of paths)
      expect(() => rustDeclarationSurface(readFileSync(path, "utf8"))).not.toThrow();
  });
});

describe("separately compiled native consumers", () => {
  it.each([
    "use usecase::model::Value;",
    "use domain as rules;",
    "use crate as root;",
    "use super::{platform, model};",
    "use crate::platform::*;",
    "use super::platform::{Services as S};",
    "crate::model::Value::new();",
    "super::super::model::Value::new();",
    "macro_rules! native { () => {} }",
    '#[path = "../model.rs"] mod model;',
    "use lens_lib::model;",
    '#[serde(from = "crate::model::Value")] struct Native;',
    "#[derive(Custom)] struct Native;",
    "hidden_consumer!();",
  ])("rejects native access outside the finite effect boundary: %s", (source) => {
    expect(nativeCompositionViolations(source).length).toBeGreaterThan(0);
  });

  it.each([
    "use super::platform::{Services, Presentation};",
    "impl<R> crate::platform::WindowPresentation<R> for Native {}",
    "pub(crate) fn run() { super::run_with_runtime(builder, services, presentation); }",
    "#[cfg_attr(mobile, tauri::mobile_entry_point)] pub fn run() {}",
  ])("admits explicit composition interfaces: %s", (source) => {
    expect(nativeCompositionViolations(source)).toEqual([]);
  });

  it.each([
    '#[cfg(target_os = "macos")] fn hidden() { usecase::value(); }',
    '#[cfg(feature = "native")] fn hidden() {}',
    '#[cfg_attr(target_os = "macos", derive(Custom))] struct Value;',
  ])("rejects common consumers hidden from Linux: %s", (source) => {
    expect(
      commonShellConditionalViolations("apps/desktop/src-tauri/src/lib.rs", source).length,
    ).toBeGreaterThan(0);
  });

  it("admits only the exact native entries at their declared path", () => {
    const source = '#[cfg(target_os = "macos")] mod native;';
    expect(commonShellConditionalViolations("apps/desktop/src-tauri/src/lib.rs", source)).toEqual(
      [],
    );
    expect(
      commonShellConditionalViolations("apps/desktop/src-tauri/src/other.rs", source),
    ).not.toEqual([]);
    expect(
      commonShellConditionalViolations(
        "apps/desktop/src-tauri/src/lib.rs",
        '#[cfg(target_os = "macos")] native::configure_activation(app, usecase::value());',
      ),
    ).not.toEqual([]);
  });

  it("allows test conditions and type-checked cfg! expressions", () => {
    expect(
      commonShellConditionalViolations(
        "common.rs",
        `
      #[cfg(test)] mod tests { fn x() {} }
      fn value() -> bool { cfg!(target_os = "macos") }
    `,
      ),
    ).toEqual([]);
  });
});
