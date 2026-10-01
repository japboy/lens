import { describe, expect, it } from "vitest";
import { webSourceViolations } from "./web-source-boundaries.ts";
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
  it.each(["adapter-math-renderer/node", "adapter-math-renderer/manifest", "fs", "path"])(
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
});
