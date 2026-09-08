import type { LensOutputBlock } from "./types";
import { imageDataUrl, type LensOutputPresentation } from "./view-model";

export interface PresentedOutputImage {
  readonly kind: "image";
  /** Snapshot-local identity. It never implies correspondence across representations. */
  readonly id: string;
  readonly source: string;
  readonly mimeType: string;
}

export interface PresentedOutputHtml {
  readonly kind: "html";
  readonly id: string;
  readonly resourceId: string;
  readonly mimeType: "text/html";
  readonly uri: string;
  readonly byteLength: number;
}

export type PresentedOutputMedia = PresentedOutputImage | PresentedOutputHtml;

export interface OutputNarrativeBlock {
  readonly block: LensOutputBlock;
  readonly index: number;
}

export interface OutputMediaComposition {
  readonly media: readonly PresentedOutputMedia[];
  readonly narrative: readonly OutputNarrativeBlock[];
}

/**
 * Standalone typed images are output artifacts; Markdown owns its inline media.
 * This product policy uses the published block contract, never DOM order or prose.
 */
export function composeOutputMedia(output: LensOutputPresentation): OutputMediaComposition {
  const media: PresentedOutputMedia[] = [];
  const narrative: OutputNarrativeBlock[] = [];
  output.blocks.forEach((block, index) => {
    const source = block.type === "image" ? imageDataUrl(block) : undefined;
    if (block.type === "image" && source) {
      media.push({
        kind: "image",
        id: `${output.identity ?? "initial"}:image:${index}`,
        source,
        mimeType: block.mime_type.toLowerCase(),
      });
    } else if (block.type === "html") {
      if (output.mode === "settled" && output.published) {
        media.push({
          kind: "html",
          id: `${output.published.operationId}:${output.published.representationId}:html:${block.resource_id}`,
          resourceId: block.resource_id,
          mimeType: block.mime_type,
          uri: block.uri,
          byteLength: block.byte_length,
        });
      }
    } else {
      narrative.push({ block, index });
    }
  });
  return { media, narrative };
}
