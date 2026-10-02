import type { DeferredDocumentBlock, DocumentBlock } from "ui/contracts/session-document";
export const PREVIEW_SCENARIOS = ["ready", "loading", "failed", "pending", "delayed"] as const;
export type PreviewScenario = (typeof PREVIEW_SCENARIOS)[number];
/** Fixed browser-owned resources exercise the real deferred document renderer. */
export function previewDocumentLoader(scenario: PreviewScenario, answer: DocumentBlock) {
  return async (request: DeferredDocumentBlock): Promise<DocumentBlock> => {
    if (request.entry_id !== "answer" || request.block_index !== 0 || request.revision !== 1)
      throw new Error("Unknown preview document resource");
    if (scenario === "failed") throw new Error("Fixture deferred body failure");
    if (scenario === "delayed") await new Promise<void>((resolve) => setTimeout(resolve, 250));
    return structuredClone(answer);
  };
}
