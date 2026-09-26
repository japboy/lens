import { composeOutputMedia } from "../output-media";
import type { SessionDocument } from "./session-document";
import type { HtmlOutputContent } from "./html-output-controller";
import type { LensOutputBlock } from "../types";
import type { LensOutputPresentation } from "../view-model";

/** Inline legacy/fixture adapter; native replay uses deferred response manifests. */
export function historyPresentation(
  document: SessionDocument | undefined,
  identity: string,
): {
  presentation: LensOutputPresentation;
  htmlContent?: HtmlOutputContent;
  htmlContents: ReadonlyMap<string, HtmlOutputContent>;
} {
  const entries = document?.entries ?? [];
  const blocks: LensOutputBlock[] = [];
  const contents = new Map<string, HtmlOutputContent>();
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
        blocks.push({
          type: "html",
          resource_id: resourceId,
          mime_type: "text/html",
          uri: `urn:lens:history:${encodeURIComponent(resourceId)}`,
          byte_length: new TextEncoder().encode(block.text).length,
        });
        contents.set(resourceId, { resourceId, status: "ready", content: block.text });
      } else if (entry.kind === "message" || block.type === "image") {
        blocks.push(block);
      }
    });
  }
  const presentation: LensOutputPresentation = {
    identity,
    artifactIdentity: identity,
    blocks,
    mode: blocks.length ? "settled" : "empty",
  };
  const htmlContents = new Map<string, HtmlOutputContent>();
  for (const media of composeOutputMedia(presentation).media) {
    if (media.kind !== "html") continue;
    const content = contents.get(media.resourceId);
    if (content) htmlContents.set(media.id, content);
  }
  return {
    presentation,
    htmlContents,
    htmlContent: contents.size === 1 ? contents.values().next().value : undefined,
  };
}
