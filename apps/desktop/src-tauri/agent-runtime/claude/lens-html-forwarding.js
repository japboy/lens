// Claude's SDK normalizes MCP results to model-facing content. Reconstruct
// only our successful echo publisher from its correlated tool-use input.
// Never interpret arbitrary tool text, resources, or Markdown links as HTML.
import { createHash as lensHtmlCreateHash } from "node:crypto";
// Invoked by the pinned upstream call site added during managed installation.
// eslint-disable-next-line no-unused-vars
function lensHtmlToolUpdate(toolResult, toolUse) {
  if (
    toolUse?.name !== "mcp__lens_output__publish_html" ||
    toolResult?.type !== "tool_result" ||
    toolResult.is_error === true ||
    typeof toolResult.tool_use_id !== "string" ||
    !toolResult.tool_use_id ||
    toolUse.id !== toolResult.tool_use_id
  )
    return undefined;
  const input = toolUse.input;
  if (
    !input ||
    typeof input !== "object" ||
    Array.isArray(input) ||
    Object.keys(input).length !== 1 ||
    typeof input.html !== "string" ||
    input.html.trim().length === 0 ||
    Buffer.byteLength(input.html, "utf8") > 512 * 1024
  )
    return undefined;
  const hash = lensHtmlCreateHash("sha256").update(toolResult.tool_use_id).digest("hex");
  const id = `${hash.slice(0, 8)}-${hash.slice(8, 12)}-${hash.slice(12, 16)}-${hash.slice(16, 20)}-${hash.slice(20, 32)}`;
  return {
    content: [
      {
        type: "content",
        content: {
          type: "resource",
          resource: {
            uri: `urn:lens:html:${id}`,
            mimeType: "text/html",
            text: input.html,
          },
        },
      },
    ],
  };
}
