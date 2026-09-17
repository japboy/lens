import notifications from "./acp-media-hero.json";
import type { DocumentBlock, SessionDocument } from "../../src/application/session-document";
import type { LensState, LensOutputBlock } from "../../src/types";
export const mediaCases = ["single", "multiple", "mixed", "none"] as const;
export type MediaCase = (typeof mediaCases)[number];
export const artifact = '<!doctype html><html><head><style>body{margin:0;background:#16324f;color:#fff;font:24px system-ui;padding:24px}strong{color:#ffdc73}</style></head><body><strong>Styled HTML</strong><p>HTML and images share the media hero.</p></body></html>';
export function mediaFixture(choice: MediaCase) {
  const images: DocumentBlock[] = notifications.slice(0, choice === "single" ? 1 : 3).map(({ params }) => ({type:"image",mime_type:"image/png",data:params.update.content.data!}));
  const media = choice === "none" ? [] : images;
  const content: DocumentBlock[] = [...media, ...(choice === "mixed" ? [{type:"html" as const,text:artifact}] : []), {type:"markdown",text:"## Shared answer\n\nThe same narrative follows the media.\n\n- First detail\n- Second detail"}];
  const document: SessionDocument = {entries:[{id:"user",kind:"message",role:"user",blocks:[{type:"markdown",text:"Show the fixture"}]},{id:"answer",kind:"message",role:"assistant",blocks:content}]};
  const blocks: LensOutputBlock[] = content.map(block => block.type === "html" ? {type:"html",resource_id:"fixture-html",mime_type:"text/html",uri:"urn:lens:fixture",byte_length:new TextEncoder().encode(block.text).length} : block as LensOutputBlock);
  const lens: LensState = {operation_id:"fixture",stage:"completed",prompt_execution_revision:0,output_blocks:blocks,representation:{prompt_execution_revision:0,representation_id:`fixture-${choice}`,context_id:"fixture",context_revision:1,projection:{revision:1,digest:"fixture"},run_id:"fixture",output_blocks:blocks}};
  return {document,lens,htmlContent:choice === "mixed" ? {resourceId:"fixture-html",status:"ready" as const,content:artifact} : undefined};
}
