import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { describe, expect, it } from "vitest";

describe("managed adapter HTML compatibility contracts", () => {
  it("forwards only successful Codex publisher HTML resources", () => {
    const context = vm.createContext({ Buffer });
    vm.runInContext(patchSource("codex"), context);
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

  it("reconstructs only correlated successful Claude publisher HTML", () => {
    const cryptoImport = 'import { createHash as lensHtmlCreateHash } from "node:crypto";';
    const source = patchSource("claude");
    expect(source).toContain(cryptoImport);
    const context = vm.createContext({ Buffer, lensHtmlCreateHash: createHash });
    vm.runInContext(source.replace(cryptoImport, ""), context);
    const forward = context.lensHtmlToolUpdate as (
      result: unknown,
      tool: unknown,
    ) => HtmlUpdate | undefined;
    const tool = {
      id: "tool-123",
      name: "mcp__lens_output__publish_html",
      input: { html: "<h1>Hello</h1>" },
    };
    const result = {
      type: "tool_result",
      tool_use_id: tool.id,
      content: [{ type: "text", text: "SDK-normalized response" }],
    };
    const actual = forward(result, tool);
    expect(actual).toBeDefined();
    expect(actual?.content[0].content.resource.text).toBe(tool.input.html);
    expect(actual?.content[0].content.resource.uri).toMatch(/^urn:lens:html:[a-f0-9-]{36}$/);
    expect(actual?.content[0].content.resource.uri).toBe(
      forward(result, tool)?.content[0].content.resource.uri,
    );
    for (const changes of [
      { name: "other" },
      { id: "mismatch" },
      { input: {} },
      { input: { html: " " } },
      { input: { html: "a".repeat(512 * 1024 + 1) } },
      { input: { html: 123 } },
      { input: { html: "x", other: true } },
    ])
      expect(forward(result, { ...tool, ...changes })).toBeUndefined();
    for (const changes of [
      { type: "tool_use" },
      { is_error: true },
      { tool_use_id: "" },
      { tool_use_id: "mismatch" },
    ])
      expect(forward({ ...result, ...changes }, tool)).toBeUndefined();
    expect(forward(result, null)).toBeUndefined();
  });
});

type HtmlUpdate = {
  content: {
    type: string;
    content: { type: string; resource: { uri: string; mimeType: string; text: string } };
  }[];
};

function patchSource(adapter: "codex" | "claude") {
  return readFileSync(
    new URL(
      `../apps/desktop/src-tauri/agent-runtime/${adapter}/lens-html-forwarding.js`,
      import.meta.url,
    ),
    "utf8",
  );
}
