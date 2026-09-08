import { readFileSync } from "node:fs";
import vm from "node:vm";
import { describe, expect, it } from "vitest";

type HtmlUpdate = {
  content: {
    type: string;
    content: { type: string; resource: { uri: string; mimeType: string; text: string } };
  }[];
};

describe("codex HTML forwarding", () => {
  it("forwards only successful Codex publisher HTML resources", () => {
    const context = vm.createContext({ Buffer });
    vm.runInContext(
      readFileSync(new URL("./lens-html-forwarding.js", import.meta.url), "utf8"),
      context,
    );
    const invoke = context.lensHtmlToolContent as (item: unknown) => Partial<HtmlUpdate>;
    const forward = (item: unknown): Partial<HtmlUpdate> =>
      JSON.parse(JSON.stringify(invoke(item)));
    const resource = {
      type: "resource",
      resource: {
        uri: "urn:lens:html:01234567-89ab-cdef-0123-456789abcdef",
        mimeType: "text/html",
        text: "<h1>Hello</h1>",
      },
    };
    const item = {
      server: "lens_output",
      tool: "publish_html",
      status: "completed",
      error: null,
      result: { content: [resource] },
    };
    expect(forward(item)).toEqual({ content: [{ type: "content", content: resource }] });
    for (const changes of [
      { server: "other" },
      { tool: "other" },
      { status: "failed" },
      { error: {} },
      { result: { isError: true, content: [resource] } },
      { result: { content: [{ type: "text", text: JSON.stringify(resource) }] } },
    ])
      expect(forward({ ...item, ...changes })).toEqual({});
    for (const changes of [
      { uri: "file:///tmp/a.html" },
      { text: "" },
      { text: "a".repeat(512 * 1024 + 1) },
      { mimeType: "image/png" },
    ])
      expect(
        forward({
          ...item,
          result: {
            content: [{ type: "resource", resource: { ...resource.resource, ...changes } }],
          },
        }),
      ).toEqual({});
    expect(forward({ ...item, result: { content: [resource, resource] } }).content).toHaveLength(1);
  });
});
