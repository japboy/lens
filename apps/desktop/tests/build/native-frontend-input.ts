import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { BUILD_PATHS } from "../../tooling/build-paths.ts";
const desktop = fileURLToPath(new URL("../../", import.meta.url));
describe("native frontend distribution contract", () => {
  it("packages only the admitted WebView output", () => {
    const config = JSON.parse(readFileSync(resolve(desktop, "src-tauri/tauri.conf.json"), "utf8"));
    expect(resolve(desktop, "src-tauri", config.build.frontendDist)).toBe(
      resolve(desktop, BUILD_PATHS.webview),
    );
  });
});
