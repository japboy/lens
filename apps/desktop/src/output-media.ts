import type { LensOutputBlock } from "./types";
import { imageDataUrl, type LensOutputPresentation } from "./view-model";

export interface PresentedOutputImage {
  readonly kind: "image";
  /** Snapshot-local identity. It never implies correspondence across representations. */
  readonly id: string;
  readonly source?: string;
  readonly mimeType: string;
  /** A prior initial-stream image retained until the committed body arrives. */
  readonly provisional?: boolean;
}

export interface PresentedOutputHtml {
  readonly kind: "html";
  readonly id: string;
  readonly resourceId: string;
  readonly mimeType: "text/html";
  readonly uri: string;
  readonly byteLength: number;
  readonly presentationSource?: import("./mcp-apps/composition").HtmlPresentationSource;
}

export interface PresentedOutputApp {
  readonly kind: "app";
  readonly id: string;
  readonly mimeType: "text/html;profile=mcp-app";
  readonly descriptor: import("adapter-mcp-apps-host").McpAppDescriptor;
}

export function presentMcpApps(
  apps: readonly import("adapter-mcp-apps-host").McpAppDescriptor[] = [],
): PresentedOutputApp[] {
  return apps.map((descriptor) => ({
    kind: "app",
    id: `app:${descriptor.id}`,
    mimeType: "text/html;profile=mcp-app",
    descriptor,
  }));
}

export type PresentedOutputMedia = PresentedOutputImage | PresentedOutputHtml | PresentedOutputApp;

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
export function composeOutputMedia(
  output: LensOutputPresentation,
  apps: readonly import("adapter-mcp-apps-host").McpAppDescriptor[] = [],
): OutputMediaComposition {
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
      if (output.mode === "settled" && (output.published || output.artifactIdentity)) {
        media.push({
          kind: "html",
          id: `${output.artifactIdentity ?? `${output.published!.operationId}:${output.published!.representationId}`}:html:${block.resource_id}`,
          resourceId: block.resource_id,
          mimeType: block.mime_type,
          uri: block.uri,
          byteLength: block.byte_length,
          presentationSource: output.published
            ? {
                kind: "live",
                output_ref: {
                  operation_id: output.published.operationId,
                  representation_id: output.published.representationId,
                },
                block_index: index,
              }
            : undefined,
        });
      }
    } else {
      narrative.push({ block, index });
    }
  });
  return { media: [...media, ...presentMcpApps(apps)], narrative };
}
