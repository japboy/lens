import type { AgentKind, LensResponseBlockDescriptor } from "../types";

export interface DeferredDocumentBlock {
  type: "deferred";
  entry_id: string;
  block_index: number;
  content_type: "markdown" | "image" | "html" | "unsupported";
  revision: number;
  byte_length: number;
  append_only?: boolean;
}
export type DocumentBlock =
  | DeferredDocumentBlock
  | { type: "markdown"; text: string }
  | { type: "image"; mime_type: string; data: string }
  | { type: "html"; text: string }
  | { type: "unsupported"; content_type: string };
export type SessionEntry =
  | { id: string; kind: "message"; role: "user" | "assistant"; blocks: DocumentBlock[] }
  | {
      id: string;
      kind: "tool";
      title: string;
      status: "pending" | "in_progress" | "completed" | "failed";
      accepted_html?: string | null;
      blocks: DocumentBlock[];
    };
export interface SessionDocument {
  entries: SessionEntry[];
}
/** Replay response grouping retains native document references, not live publications. */
export type HistoryResponseBlockDescriptor = LensResponseBlockDescriptor & {
  source: DeferredDocumentBlock;
};
export interface HistoryResponseManifest {
  response_id: string;
  sequence: number;
  blocks: HistoryResponseBlockDescriptor[];
}
export interface HistoryInterpretation {
  responses: HistoryResponseManifest[];
}
export interface SessionView {
  revision: number;
  generation?: string;
  conversation?: SessionDocument;
  patch?: { base_revision: number; index: number; entry: SessionEntry };
  phase: "idle" | "live" | "loading" | "ready" | "failed";
  agent?: AgentKind;
  session_id?: string;
  title?: string;
  interpretation?: HistoryInterpretation;
  /** Legacy inline fixtures; native replay uses deferred interpretation manifests. */
  document?: SessionDocument;
  error?: string;
}
export function isHistoryView(view: SessionView | undefined): boolean {
  return view?.phase === "loading" || view?.phase === "ready" || view?.phase === "failed";
}
