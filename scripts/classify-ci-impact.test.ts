import { describe, expect, it } from "vitest";
import { classifyCiImpact } from "./classify-ci-impact.ts";

describe("CI impact classification", () => {
  it.each([
    [["src/lens-app.ts"], "portable-only"],
    [["README.md", "src/styles/global.css", "scripts/check-product-identity.ts"], "portable-only"],
    [["src-tauri/src/lib.rs"], "native-or-control-plane"],
    [["apps/desktop/src/lens-app.ts"], "native-or-control-plane"],
    [["apps/desktop/src-tauri/tauri.macos.conf.json"], "native-or-control-plane"],
    [["package.json"], "native-or-control-plane"],
    [["Cargo.toml"], "native-or-control-plane"],
    [["Cargo.lock"], "native-or-control-plane"],
    [[".github/workflows/code-quality.yml"], "native-or-control-plane"],
    [["scripts/classify-ci-impact.ts"], "native-or-control-plane"],
    [["src/lens-app.ts", "src-tauri/Cargo.toml"], "native-or-control-plane"],
    [["an-unclassified-path"], "native-or-control-plane"],
    [[], "native-or-control-plane"],
  ] as const)("classifies %j as %s", (changedPaths, expected) => {
    expect(classifyCiImpact(changedPaths)).toBe(expected);
  });

  it("keeps native verification for a rename from a native into a portable path", () => {
    expect(classifyCiImpact(["src-tauri/icons/128x128.png", "src/128x128.png"])).toBe(
      "native-or-control-plane",
    );
  });
});
