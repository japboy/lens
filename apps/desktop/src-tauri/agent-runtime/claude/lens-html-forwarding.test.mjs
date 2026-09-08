import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import vm from "node:vm";
import assert from "node:assert/strict";
const code = readFileSync(new URL("./lens-html-forwarding.js", import.meta.url), "utf8").replace(
  'import { createHash as lensHtmlCreateHash } from "node:crypto";',
  "",
);
const context = vm.createContext({ Buffer, lensHtmlCreateHash: createHash });
vm.runInContext(code, context);
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
const forward = (r = result, t = tool) => context.lensHtmlToolUpdate(r, t);
const actual = forward();
assert.equal(actual.content[0].content.resource.text, tool.input.html);
assert.match(actual.content[0].content.resource.uri, /^urn:lens:html:[a-f0-9-]{36}$/);
assert.equal(actual.content[0].content.resource.uri, forward().content[0].content.resource.uri);
for (const changes of [
  { name: "other" },
  { id: "mismatch" },
  { input: {} },
  { input: { html: " " } },
  { input: { html: "a".repeat(512 * 1024 + 1) } },
  { input: { html: 123 } },
  { input: { html: "x", other: true } },
])
  assert.equal(forward(result, { ...tool, ...changes }), undefined);
for (const changes of [
  { type: "tool_use" },
  { is_error: true },
  { tool_use_id: "" },
  { tool_use_id: "mismatch" },
])
  assert.equal(forward({ ...result, ...changes }), undefined);
assert.equal(forward(result, null), undefined);
console.log("Lens Claude HTML compatibility reconstruction checks passed");
