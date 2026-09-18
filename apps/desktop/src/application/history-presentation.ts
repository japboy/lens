import type { SessionDocument } from "./session-document";
import type { HtmlOutputContent } from "./html-output-controller";
import type { LensOutputBlock } from "../types";
import type { LensOutputPresentation } from "../view-model";

/** Render the backend-selected history result without selecting a turn again. */
export function historyPresentation(
  document: SessionDocument | undefined,
  identity: string,
): {
  presentation: LensOutputPresentation;
  htmlContent?: HtmlOutputContent;
} {
  const entries = document?.entries ?? [];
  const blocks: LensOutputBlock[] = [];
  let htmlContent: HtmlOutputContent | undefined;
  for (const entry of entries) {
    if (entry.kind === "tool" && entry.status !== "completed") continue;
    if (entry.kind === "message" && entry.role !== "assistant") continue;
    const content = [...entry.blocks];
    if (entry.kind === "tool" && entry.accepted_html)
      content.push({ type: "html", text: entry.accepted_html });
    content.forEach((block, index) => {
      if (block.type === "deferred") return;
      if (block.type === "html") {
        const resourceId = `${identity}:${entry.id}:${index}`;
        // The normal media presenter supports one HTML artifact: retain the latest.
        for (let i = blocks.length - 1; i >= 0; i--)
          if (blocks[i]?.type === "html") blocks.splice(i, 1);
        blocks.push({
          type: "html",
          resource_id: resourceId,
          mime_type: "text/html",
          uri: `urn:lens:history:${encodeURIComponent(resourceId)}`,
          byte_length: new TextEncoder().encode(block.text).length,
        });
        htmlContent = { resourceId, status: "ready", content: block.text };
      } else if (entry.kind === "message" || block.type === "image") {
        blocks.push(block);
      }
    });
  }
  return {
    presentation: {
      identity,
      artifactIdentity: identity,
      blocks,
      mode: blocks.length ? "settled" : "empty",
    },
    htmlContent,
  };
}
