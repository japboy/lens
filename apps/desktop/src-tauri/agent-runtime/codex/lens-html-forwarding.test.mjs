import { readFileSync } from "node:fs";
import vm from "node:vm";
import assert from "node:assert/strict";
const code = readFileSync(new URL("./lens-html-forwarding.js", import.meta.url), "utf8");
const context = vm.createContext({ Buffer });
vm.runInContext(code, context);
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
const forward = (value) => JSON.parse(JSON.stringify(context.lensHtmlToolContent(value)));
assert.deepEqual(forward(item), { content: [{ type: "content", content: resource }] });
for (const changes of [
  { server: "other" },
  { tool: "other" },
  { status: "failed" },
  { error: {} },
  { result: { isError: true, content: [resource] } },
  { result: { content: [{ type: "text", text: JSON.stringify(resource) }] } },
])
  assert.deepEqual(forward({ ...item, ...changes }), {});
for (const changes of [
  { uri: "file:///tmp/a.html" },
  { text: "" },
  { text: "a".repeat(512 * 1024 + 1) },
  { mimeType: "image/png" },
])
  assert.deepEqual(
    forward({
      ...item,
      result: { content: [{ type: "resource", resource: { ...resource.resource, ...changes } }] },
    }),
    {},
  );
assert.equal(forward({ ...item, result: { content: [resource, resource] } }).content.length, 1);
console.log("Lens Codex HTML forwarding checks passed");
