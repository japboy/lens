import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { assertGenerationOutput, BUILD_PATHS } from "./build-paths.ts";

describe("generated output ownership", () => {
  const desktop = resolve(".");
  it.each([
    BUILD_PATHS.webview,
    BUILD_PATHS.tests,
    BUILD_PATHS.tests + "/repro",
    BUILD_PATHS.development + "/run-1/build-1",
  ])("admits complete output at %s", (path) => {
    expect(() => assertGenerationOutput(desktop, resolve(desktop, path))).not.toThrow();
  });

  it.each([
    ".",
    ".build",
    BUILD_PATHS.staging,
    ".build/cache/typescript",
    BUILD_PATHS.development,
    ".build/tests/../cache",
    ".build/tests-other",
    "../outside",
    "src",
  ])("rejects destructive output replacement at %s", (path) => {
    expect(() => assertGenerationOutput(desktop, resolve(desktop, path))).toThrow("Output must");
  });

  it("packages only the admitted WebView output", () => {
    const config = JSON.parse(readFileSync(resolve("src-tauri/tauri.conf.json"), "utf8"));
    expect(resolve("src-tauri", config.build.frontendDist)).toBe(resolve(BUILD_PATHS.webview));
  });
});
