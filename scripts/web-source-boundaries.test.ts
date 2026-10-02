import { describe, expect, it } from "vitest";
import {
  crossPackageRelativeImportViolations,
  webSourceViolations,
} from "./web-source-boundaries.ts";
const owner = "packages/adapter-mcp-apps-host";
const path = `${owner}/src/controller.ts`;
describe("framework-free web package boundaries", () => {
  it.each([
    'import {invoke} from "@tauri-apps/api/core"',
    'export * from "../../../apps/desktop/src/types"',
    'type Port = import("desktop/types").Port',
    'await import("adapter-math-renderer")',
    'await import("adapter-mcp-apps-view")',
    'require("node:fs")',
    "await import(dynamicPath)",
    'export * from "./node/assets"',
    'import { fixture } from "../tests/protocol/handshake.ts"',
    'export * from "./controller.test.ts"',
  ])("rejects native, reverse, renderer and dynamic source escape: %s", (source) => {
    expect(webSourceViolations(path, source, owner)).not.toEqual([]);
  });
  it("admits local generic contracts and SDK imports", () => {
    expect(
      webSourceViolations(
        path,
        'import {AppBridge} from "@modelcontextprotocol/ext-apps/app-bridge"; export * from "./types"',
        owner,
      ),
    ).toEqual([]);
  });
  it.each(["adapter-math-renderer/node", "adapter-math-renderer/html-math-manifest", "fs", "path"])(
    "rejects Node export aliases and bare built-ins from renderer browser: %s",
    (specifier) => {
      expect(
        webSourceViolations(
          "packages/adapter-math-renderer/src/index.ts",
          `export * from "${specifier}"`,
          "packages/adapter-math-renderer",
        ),
      ).not.toEqual([]);
    },
  );
  it("keeps Node dependencies in the explicit node facet", () => {
    expect(
      webSourceViolations(
        "packages/adapter-math-renderer/src/node/assets.ts",
        'import fs from "node:fs"; import {build} from "vite"',
        "packages/adapter-math-renderer",
      ),
    ).toEqual([]);
  });
  it.each([
    "@tauri-apps/api/core",
    "desktop/types",
    "../../../apps/desktop/src/types",
    "adapter-lit-prerenderer",
    "adapter-lit-prerenderer/verify",
    "node:fs",
    "virtual:lens-html-math-assets",
    "ui/test-fixtures/media-parity",
  ])("keeps the UI browser package free of application and build authority: %s", (specifier) => {
    expect(
      webSourceViolations(
        "packages/ui/src/index.ts",
        `export * from "${specifier}"`,
        "packages/ui",
      ),
    ).not.toEqual([]);
  });
  it("admits declared Node packages without admitting reverse native dependencies", () => {
    const path = "packages/adapter-lit-prerenderer/src/generate.ts";
    expect(
      webSourceViolations(
        path,
        'import {build} from "vite"; import fs from "node:fs"',
        "packages/adapter-lit-prerenderer",
        "node",
      ),
    ).toEqual([]);
    expect(
      webSourceViolations(
        path,
        'import "../../../apps/desktop/tooling/build-paths.ts"',
        "packages/adapter-lit-prerenderer",
        "node",
      ),
    ).not.toEqual([]);
  });
});

describe("public exports across all shared package consumers", () => {
  const packages = [
    "packages/ui",
    "packages/adapter-lit-prerenderer",
    "packages/adapter-math-renderer",
  ];
  it.each([
    ["apps/desktop/src/view.ts", "../../../packages/ui/src/contracts/lens.ts"],
    ["apps/desktop/tooling/check.ts", "../../../packages/adapter-lit-prerenderer/src/verify.ts"],
    ["tests/rendering/contract.ts", "../../packages/ui/src/contracts/lens.ts"],
    ["scripts/generation.ts", "../packages/adapter-math-renderer/src/node/html-math-manifest.ts"],
    ["packages/ui/tests/fixture.ts", "../../adapter-lit-prerenderer/src/verify.ts"],
    ["apps/desktop/src/view.ts", "../../../packages/ui/../ui/src/contracts/lens.ts"],
  ])("rejects a cross-package source import in %s", (path, specifier) => {
    for (const source of [
      `export * from "${specifier}"`,
      `await import("${specifier}")`,
      `require("${specifier}")`,
    ])
      expect(crossPackageRelativeImportViolations(path, source, packages)).not.toEqual([]);
  });
  it("rejects escaped module names and imports inside actual template interpolation", () => {
    for (const source of [
      'import "../../../packages/u\\u0069/src/private.ts"',
      '`value:${import("../../../packages/ui/src/private.ts")}`',
    ])
      expect(
        crossPackageRelativeImportViolations("apps/desktop/src/view.ts", source, packages),
      ).not.toEqual([]);
  });
  it("retains local package modules, public imports, and repository app orchestration", () => {
    for (const [path, source] of [
      ["packages/ui/tests/fixture.ts", 'import "../src/contracts/lens.ts"'],
      ["apps/desktop/src/view.ts", 'import "ui/contracts/lens"'],
      ["scripts/frontend-artifact.ts", 'import "../apps/desktop/tooling/build-paths.ts"'],
      ["tests/contract.ts", 'const fixture = `import "../packages/ui/private"`;'],
    ])
      expect(crossPackageRelativeImportViolations(path, source, packages)).toEqual([]);
  });
});
