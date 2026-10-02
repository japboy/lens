import type { LensResponseBlockDescriptor, LensDeliveryCoverage, LensOutputBlock } from "./lens";
import type { DeferredDocumentBlock } from "./session-document";
import type { PresentedOutputMedia } from "../presentation/output-media";
export type ResponseBlockDescriptor = LensResponseBlockDescriptor & {
  source?: DeferredDocumentBlock;
};
export interface ResponseManifest {
  mcpApps?: readonly import("adapter-mcp-apps-host").McpAppDescriptor[];
  id: string;
  sequence: number;
  delivery?: LensDeliveryCoverage;
  blocks: readonly ResponseBlockDescriptor[];
}

export interface ResponseHistoryPresentation {
  scopeId: string;
  responses: readonly ResponseManifest[];
  media: readonly PresentedOutputMedia[];
  mediaErrors: ReadonlyMap<string, string>;
  capacityReached: boolean;
  /** Previously displayed initial output; never authoritative committed bodies. */
  provisionalBlocks?: ReadonlyMap<string, ProvisionalResponseBlock>;
}
export interface ProvisionalResponseBlock {
  body: LensOutputBlock;
  bytes: number;
  release: () => void;
}
export type LoadResponseBlock = (
  operationId: string,
  representationId: string,
  blockIndex: number,
) => Promise<LensOutputBlock>;
export interface ResponseBodyPort {
  getResponseBlock(
    operationId: string,
    representationId: string,
    blockIndex: number,
  ): Promise<LensOutputBlock>;
}
