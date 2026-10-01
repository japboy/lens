import { describe, expect, it } from "vitest";
import { sourceInclusionViolations } from "./rust-source-boundaries.ts";

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
          include(`adapter-mcp-apps-web/src/assets/${asset}`),
          "apps/desktop",
        ),
      ).toEqual([]);
    },
  );
  it("admits the canonical built-in renderer shell only to its Agent owner", () => {
    expect(
      sourceInclusionViolations(
        root,
        "apps/desktop/src-tauri/src/agent.rs",
        include("adapter-rich-content-web/src/assets/rich-html-app.html"),
        "apps/desktop",
      ),
    ).toEqual([]);
  });
  it.each([
    [host, "adapter-mcp-apps-web/src/controller.ts", "include_str"],
    [host, "adapter-rich-content-web/src/assets/rich-html-app.html", "include_str"],
    [host, "adapter-mcp-apps-web/src/assets/sandbox-proxy.js", "include_bytes"],
    [
      "apps/desktop/src-tauri/src/other.rs",
      "adapter-mcp-apps-web/src/assets/sandbox-proxy.js",
      "include_str",
    ],
  ])("rejects undeclared source/asset ownership %s %s %s", (path, asset, macro) => {
    expect(
      sourceInclusionViolations(root, path, include(asset, macro), "apps/desktop"),
    ).not.toEqual([]);
  });
});
