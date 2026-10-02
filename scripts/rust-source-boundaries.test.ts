import { describe, expect, it } from "vitest";
import {
  portableSourceViolations,
  rustTokens,
  sourceInclusionViolations,
} from "./rust-source-boundaries.ts";
const root = "/workspace";

describe("portable source escape restrictions", () => {
  it.each([
    '#[cfg(target_os = "macos")] fn rule() {}',
    '#[cfg(any(test, target_arch = "aarch64"))] fn rule() {}',
    '#[cfg_attr(test, path = "../native.rs")] mod native;',
    'extern "C" { fn native(); }',
    "unsafe { native(); }",
    'std /* nested /* comment */ still comment */ :: fs::read("input");',
    "use std::{fs as disk};",
    "use std::r#process::Command;",
    "use std::time::Instant;",
    "let id = uuid::Uuid::new_v4();",
    'include!(concat!(env!("OUT_DIR"), "/native.rs"));',
    "macro_rules! inject { () => { native() } }",
    "#[unreviewed::expand] fn rule() {}",
    "unknown_macro!();",
    "windows::Win32::native();",
  ])("rejects %s", (source) => {
    expect(portableSourceViolations(source).length).toBeGreaterThan(0);
  });

  it("keeps strings, chars, lifetimes, raw strings and nested comments distinct from code", () => {
    const source = `/* outer /* unsafe */ extern */
      fn echo<'a>(value: &'a str) -> &'a str { value }
      let quote = '\\'';
      let escaped = "\\" unsafe extern fs";
      let raw = br##" /* unsafe */ " # "##;
      let windows = vec![1];
      #[cfg(test)] mod tests { #[test] fn works() { assert!(true); } }
    `;
    expect(portableSourceViolations(source)).toEqual([]);
    expect(rustTokens(source).some((token) => !token.literal && token.text === "unsafe")).toBe(
      false,
    );
  });

  it.each(['r##"unfinished"#', '"unfinished', "/* unterminated", "fn \u65e5\u672c\u8a9e() {}"])(
    "fails closed on unsupported or incomplete lexical input: %s",
    (source) => {
      expect(() => portableSourceViolations(source)).toThrow(/Unterminated|Non-ASCII/u);
    },
  );

  it("admits only the two root license documents as desktop text resources", () => {
    const check = (source: string) =>
      sourceInclusionViolations(
        root,
        "apps/desktop/src-tauri/src/about.rs",
        source,
        "apps/desktop",
      );
    expect(check('include_str!("../../../../LICENSE");')).toEqual([]);
    expect(check('include_str!("../../../../NOTICE");')).toEqual([]);
    expect(check('include_str!("../../../../Cargo.toml");')).not.toEqual([]);
    expect(check('include_bytes!("../../../../LICENSE");')).not.toEqual([]);
    expect(
      sourceInclusionViolations(
        root,
        "packages/domain/src/lib.rs",
        'include_str!("../../../LICENSE");',
        "packages/domain",
      ),
    ).not.toEqual([]);
  });

  it("rejects cross-owner and computed source/resource inclusion", () => {
    const check = (source: string) =>
      sourceInclusionViolations(root, "packages/domain/src/lib.rs", source, "packages/domain");
    for (const source of [
      'include!("own.rs");',
      'include_str!("../../usecase/src/model.rs");',
      'include_bytes!(concat!("../", "resource"));',
      '#[path = "../../usecase/src/model.rs"] mod other;',
      '#[cfg_attr(test, path = "../../usecase/src/model.rs")] mod other;',
    ])
      expect(check(source).length).toBeGreaterThan(0);
    expect(check('include_str!("../resource.txt");')).toEqual([]);
    expect(check('#[path = "other.rs"] mod other;')).toEqual([]);
    expect(portableSourceViolations('#[path = "other.rs"] mod other;')).not.toEqual([]);
  });
});

describe("explicit native shared web resource ownership", () => {
  const root = "/workspace";
  const host = "apps/desktop/src-tauri/src/mcp_apps.rs";
  const include = (asset: string, macro = "include_str") =>
    `${macro}!("../../../../packages/${asset}")`;
  it.each(["sandbox-proxy.html", "sandbox-proxy.js"])(
    "admits only canonical Host asset %s",
    (asset) => {
      expect(
        sourceInclusionViolations(
          root,
          host,
          include(`adapter-mcp-apps-host/src/assets/${asset}`),
          "apps/desktop",
        ),
      ).toEqual([]);
    },
  );
  it("admits the canonical built-in renderer shell to its Agent broker", () => {
    expect(
      sourceInclusionViolations(
        root,
        "apps/desktop/src-tauri/src/agent.rs",
        include("adapter-mcp-apps-view/src/assets/rich-html-app.html"),
        "apps/desktop",
      ),
    ).toEqual([]);
  });
  it("admits the single shared UI catalog only to the native Agent icon consumer", () => {
    const path = "apps/desktop/src-tauri/src/agent_icons.rs";
    const source = include("ui/src/assets/agent-icons.json");
    expect(sourceInclusionViolations(root, path, source, "apps/desktop")).toEqual([]);
    expect(sourceInclusionViolations(root, host, source, "apps/desktop")).not.toEqual([]);
    expect(
      sourceInclusionViolations(
        root,
        path,
        include("ui/src/assets/agent-icons.json", "include_bytes"),
        "apps/desktop",
      ),
    ).not.toEqual([]);
    expect(
      sourceInclusionViolations(root, path, include("ui/src/contracts/lens.ts"), "apps/desktop"),
    ).not.toEqual([]);
  });
  it.each([
    ["rich-html-app.html", "include_str"],
    ["rich-html-links.js", "include_str"],
    ["rich-html-links.js", "include_bytes"],
  ])("admits the canonical HTML display asset %s through %s", (asset, macro) => {
    expect(
      sourceInclusionViolations(
        root,
        host,
        include(`adapter-mcp-apps-view/src/assets/${asset}`, macro),
        "apps/desktop",
      ),
    ).toEqual([]);
  });
  it.each([
    [host, "adapter-mcp-apps-host/src/controller.ts", "include_str"],
    [
      "apps/desktop/src-tauri/src/agent.rs",
      "adapter-mcp-apps-view/src/assets/rich-html-links.js",
      "include_str",
    ],
    [host, "adapter-mcp-apps-view/src/assets/rich-html-app.html", "include_bytes"],
    [host, "adapter-mcp-apps-host/src/assets/sandbox-proxy.js", "include_bytes"],
    [
      "apps/desktop/src-tauri/src/other.rs",
      "adapter-mcp-apps-host/src/assets/sandbox-proxy.js",
      "include_str",
    ],
  ])("rejects undeclared source/asset ownership %s %s %s", (path, asset, macro) => {
    expect(
      sourceInclusionViolations(root, path, include(asset, macro), "apps/desktop"),
    ).not.toEqual([]);
  });
});
