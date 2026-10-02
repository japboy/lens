import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { classifyChange } from "./ci-plan.ts";
import { nativeInputInventory, rustIncludeInputs } from "./ci-native-inputs.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const tracked = execFileSync(
  "git",
  ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
  { cwd: root, encoding: "utf8" },
)
  .split("\0")
  .filter((path) => path && existsSync(resolve(root, path)));
const sources = new Map(
  tracked
    .filter((path) => /^(?:apps|packages)\/.+\.rs$/u.test(path))
    .map((path) => [path, readFileSync(resolve(root, path), "utf8")]),
);

function requireNativeInput(path: string): void {
  const { requirements } = classifyChange({ path, before: "", after: "" });
  if (!requirements.sharedRust || requirements.macos === "none")
    throw new Error(`Native input lacks verification: ${path}`);
}

describe("checked native consumer inventory", () => {
  it("keeps every literal include and reviewed build input under native verification", () => {
    const inventory = nativeInputInventory(sources);
    expect([...inventory.values()].flat().length).toBeGreaterThan(20);
    for (const inputs of inventory.values())
      for (const path of inputs) {
        if (!existsSync(resolve(root, path))) throw new Error(`Missing native input: ${path}`);
        requireNativeInput(path);
      }
  });

  it("detects a new native consumer of a frontend-only file", () => {
    const changed = new Map(sources);
    changed.set(
      "apps/desktop/src-tauri/src/future.rs",
      'const X: &str = include_str!("../../src/styles.css");',
    );
    const inventory = nativeInputInventory(changed);
    expect(() =>
      inventory.get("apps/desktop/src-tauri/src/future.rs")!.forEach(requireNativeInput),
    ).toThrow("Native input lacks verification");
  });

  it("rejects unreviewed computed includes, escaped paths and build input changes", () => {
    for (const source of [
      'include_str!(concat!("../", "file"))',
      'include_bytes!(env!("INPUT"))',
      'include!(r#"file.rs"#)',
      "use std::include_str as load;",
    ])
      expect(() => rustIncludeInputs("apps/owner.rs", source)).toThrow(
        "Unreviewed Rust include expression",
      );
    expect(() => rustIncludeInputs("apps/owner.rs", 'include_str!("../../outside")')).toThrow(
      "escapes repository",
    );
    expect(() => rustIncludeInputs("apps/owner.rs", 'include_str!("/outside")')).toThrow(
      "escapes repository",
    );
    const changed = new Map(sources);
    changed.set("packages/future/build.rs", "fn main() {}");
    expect(() => nativeInputInventory(changed)).toThrow("Unreviewed native build owner");
    changed.delete("packages/future/build.rs");
    changed.set(
      "apps/desktop/src-tauri/build.rs",
      `${sources.get("apps/desktop/src-tauri/build.rs")}\nconst INPUT: &str = "new-file";`,
    );
    expect(() => nativeInputInventory(changed)).toThrow("inventory requires review");
  });

  it("ignores comments and strings without hiding actual multiline macro invocations", () => {
    expect(
      rustIncludeInputs(
        "apps/owner.rs",
        '// include_str!("ignored")\nconst S: &str = r#"include_bytes!("ignored")"#;\ninclude_str!(\n "../LICENSE",\n)',
      ),
    ).toEqual(["LICENSE"]);
  });
});
