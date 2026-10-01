import { describe, expect, it } from "vitest";
import { webSourceViolations } from "./web-source-boundaries.ts";
const owner = "packages/adapter-mcp-apps-host";
const path = `${owner}/src/controller.ts`;
describe("framework-free web package boundaries", () => {
  it.each([
    'import {invoke} from "@tauri-apps/api/core"',
    'export * from "../../../apps/desktop/src/types"',
    'type Port = import("desktop/types").Port',
    'await import("adapter-rich-content-web")',
    'await import("adapter-mcp-apps-view")',
    'require("node:fs")',
    "await import(dynamicPath)",
    'export * from "./node/assets"',
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
  it.each(["adapter-rich-content-web/node", "adapter-rich-content-web/manifest", "fs", "path"])(
    "rejects Node export aliases and bare built-ins from renderer browser: %s",
    (specifier) => {
      expect(
        webSourceViolations(
          "packages/adapter-rich-content-web/src/index.ts",
          `export * from "${specifier}"`,
          "packages/adapter-rich-content-web",
        ),
      ).not.toEqual([]);
    },
  );
  it("keeps Node dependencies in the explicit node facet", () => {
    expect(
      webSourceViolations(
        "packages/adapter-rich-content-web/src/node/assets.ts",
        'import fs from "node:fs"; import {build} from "vite"',
        "packages/adapter-rich-content-web",
      ),
    ).toEqual([]);
  });
});
