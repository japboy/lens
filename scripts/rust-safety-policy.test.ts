import { spawnSync } from "node:child_process";
import { parse as parseToml } from "@iarna/toml";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { unsafeExceptionViolations, unsafeManifestViolations } from "./rust-source-boundaries.ts";
import { nativeCompositionViolations } from "./rust-source-surface.ts";

describe("Rust safety policy", () => {
  it.each([
    '#![allow(unsafe_code, reason = "crate bypass")]',
    "#[allow(unsafe_code)] unsafe { operation() }",
    '#[allow(unsafe_code, reason = "")] unsafe { operation() }',
    '#[allow(unsafe_code, reason = "module bypass")] mod native {}',
    '#[allow(unsafe_code, reason = "function bypass")] fn safe() {}',
    '#[cfg_attr(test, allow(unsafe_code, reason = "conditional bypass"))] unsafe {}',
    '#[allow(unsafe_op_in_unsafe_fn, reason = "implicit operation")] unsafe fn callback() {}',
    '#[expect(unsafe_code, reason = "suppressed")] unsafe {}',
    '#[allow(unsafe_code, reason = "caller contract")] unsafe fn callback() { unsafe {} }',
  ])("rejects an undocumented or broad exception: %s", (source) => {
    expect(unsafeExceptionViolations(source)).not.toEqual([]);
  });

  it("admits documented operation and ABI declarations without reading strings as code", () => {
    expect(
      unsafeExceptionViolations(`
      const EXAMPLE: &str = "#![allow(unsafe_code)]";
      #[allow(unsafe_code, reason = "[boundary:ax-ffi] FFI retains no borrowed pointer after return")]
      unsafe { native_call() }
      #[allow(unsafe_code, reason = "[boundary:ax-ffi] C ABI requires callback pointer ownership contract")]
      unsafe extern "C" fn callback() {}
    `),
    ).toEqual([]);
  });

  it("preserves native composition restrictions around narrowly reviewed exceptions", () => {
    const reviewed =
      '#[allow(unsafe_code, reason = "[boundary:ax-ffi] Live main-thread handle for this call")] unsafe {}';
    expect(nativeCompositionViolations(reviewed)).toEqual([]);
    expect(nativeCompositionViolations("#[allow(unsafe_code)] unsafe {}")).not.toEqual([]);
    expect(
      nativeCompositionViolations('#[allow(unsafe_code, reason = "all")] mod native {}'),
    ).not.toEqual([]);
    expect(nativeCompositionViolations("#[allow(unused)] fn safe() {}")).not.toEqual([]);
  });

  it("requires inherited lints and rejects local overrides", () => {
    const check = (source: string, root = false) =>
      unsafeManifestViolations(parseToml(source), root);
    expect(check("[lints]\nworkspace = true\n")).toEqual([]);
    expect(check('[package]\nname = "bypass"\n')).not.toEqual([]);
    expect(check("[lints]\nworkspace = false\n")).not.toEqual([]);
    expect(check('[workspace.lints.rust]\nunsafe_code = "warn"\n', true)).not.toEqual([]);
    expect(check('[package]\nmetadata = """\n[lints]\nworkspace = true\n"""\n')).not.toEqual([]);
  });

  it("Cargo rejects arbitrary unsafe across targets and accepts only explicit safe operation scopes", () => {
    const root = mkdtempSync(join(tmpdir(), "lens-rust-safety-"));
    const policy = readFileSync("Cargo.toml", "utf8").match(
      /\[workspace\.lints\.rust\]\n[^[]+/u,
    )![0];
    try {
      mkdirSync(join(root, "member", "src"), { recursive: true });
      writeFileSync(
        join(root, "Cargo.toml"),
        '[workspace]\nmembers = ["member"]\nresolver = "2"\n' + policy,
      );
      writeFileSync(
        join(root, "member", "Cargo.toml"),
        '[package]\nname = "safety-probe"\nversion = "0.0.0"\nedition = "2021"\n[lints]\nworkspace = true\n',
      );
      const source = join(root, "member", "src", "lib.rs");
      const check = () =>
        spawnSync("cargo", ["check", "--offline", "--all-targets"], {
          cwd: root,
          encoding: "utf8",
          timeout: 30000,
        });
      writeFileSync(source, "pub fn arbitrary() { unsafe { std::ptr::read(&1); } }");
      const denied = check();
      expect(denied.status).not.toBe(0);
      expect(denied.stderr).toContain("unsafe");
      writeFileSync(
        source,
        `pub fn reviewed() {
        let value = 1;
        #[allow(unsafe_code, reason = "Pointer references a live initialized local for this read")]
        unsafe { std::ptr::read(&value); }
      }`,
      );
      expect(check().status).toBe(0);
      writeFileSync(
        source,
        `#[allow(unsafe_code, reason = "Caller provides a readable initialized pointer")]
        pub unsafe fn implicit(pointer: *const u8) -> u8 { *pointer }`,
      );
      const implicit = check();
      expect(implicit.status).not.toBe(0);
      expect(implicit.stderr).toMatch(/unsafe[-_]op[-_]in[-_]unsafe[-_]fn/u);
      writeFileSync(source, "pub fn safe() {}");
      writeFileSync(
        join(root, "member", "build.rs"),
        "fn main() { unsafe { std::ptr::read(&1); } }",
      );
      expect(check().status).not.toBe(0);
      rmSync(join(root, "member", "build.rs"));
      mkdirSync(join(root, "member", "tests"));
      writeFileSync(
        join(root, "member", "tests", "probe.rs"),
        "#[test] fn arbitrary() { unsafe { std::ptr::read(&1); } }",
      );
      expect(check().status).not.toBe(0);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }, 30000);
});
