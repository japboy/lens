import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { describe, expect, it } from "vitest";

type HtmlUpdate = {
  content: {
    type: string;
    content: { type: string; resource: { uri: string; mimeType: string; text: string } };
  }[];
};

describe("claude HTML forwarding", () => {
  it("reconstructs only correlated successful Claude publisher HTML", () => {
    const cryptoImport = 'import { createHash as lensHtmlCreateHash } from "node:crypto";';
    const source = readFileSync(new URL("./lens-html-forwarding.js", import.meta.url), "utf8");
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
