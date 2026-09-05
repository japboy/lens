import type { LensOutputBlock } from "./types";
import { imageDataUrl, type LensOutputPresentation } from "./view-model";

export interface PresentedOutputImage {
  /** Snapshot-local identity. It never implies correspondence across representations. */
  readonly id: string;
  readonly source: string;
  readonly mimeType: string;
}

export interface OutputNarrativeBlock {
  readonly block: LensOutputBlock;
  readonly index: number;
}

export interface OutputMediaComposition {
  readonly media: readonly PresentedOutputImage[];
  readonly narrative: readonly OutputNarrativeBlock[];
}

/**
 * Standalone typed images are output artifacts; Markdown owns its inline media.
 * This product policy uses the published block contract, never DOM order or prose.
 */
export function composeOutputMedia(output: LensOutputPresentation): OutputMediaComposition {
  const media: PresentedOutputImage[] = [];
  const narrative: OutputNarrativeBlock[] = [];
  output.blocks.forEach((block, index) => {
    const source = block.type === "image" ? imageDataUrl(block) : undefined;
    if (block.type === "image" && source) {
      media.push({
        id: `${output.identity ?? "initial"}:image:${index}`,
        source,
        mimeType: block.mime_type.toLowerCase(),
      });
    } else {
      narrative.push({ block, index });
    }
  });
  return { media, narrative };
}
