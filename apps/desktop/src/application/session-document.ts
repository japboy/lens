import type { AgentKind } from "../types";

export type DocumentBlock =
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
export interface SessionView {
  revision: number;
  phase: "idle" | "live" | "loading" | "ready" | "failed";
  agent?: AgentKind;
  session_id?: string;
  title?: string;
  document?: SessionDocument;
  error?: string;
}
export function isHistoryView(view: SessionView | undefined): boolean {
  return view?.phase === "loading" || view?.phase === "ready" || view?.phase === "failed";
}
