import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { HistoryIntent } from "ui/contracts/events";
import type { HistoryPage, HistoryPresentation, HistoryResult } from "ui/contracts/view-models";

export const HISTORY_PAGE_SIZE = 50;
export interface HistoryActions {
  listHistory(offset: number, limit: number): Promise<HistoryPage>;
  openHistory(id: string, revision: number): Promise<void>;
  deleteHistory(id: string, revision: number): Promise<HistoryResult>;
  deleteAllHistory(revision: number): Promise<HistoryResult>;
  syncHistory(): Promise<HistoryResult>;
  confirmAction(message: string, title: string): Promise<boolean>;
}

/** Local cache reads and explicit Agent operations have independent finite states. */
export class HistoryController implements ReactiveController {
  state: HistoryPresentation = { stage: "inactive", offset: 0, operation: "idle" };
  private connected = false;
  private lifecycle = 0;
  private readGeneration = 0;

  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly actions: HistoryActions,
  ) {
    host.addController(this);
  }

  hostConnected(): void {
    this.connected = true;
  }

  hostDisconnected(): void {
    this.connected = false;
    this.lifecycle += 1;
    this.readGeneration += 1;
    this.state = { stage: "inactive", offset: 0, operation: "idle" };
  }

  private set(next: HistoryPresentation): void {
    this.state = next;
    this.host.requestUpdate();
  }

  async load(offset: number): Promise<void> {
    if (!this.connected || this.state.operation !== "idle") return;
    const generation = ++this.readGeneration;
    const lifecycle = this.lifecycle;
    const current = () =>
      this.connected && lifecycle === this.lifecycle && generation === this.readGeneration;
    offset = Math.max(0, Math.floor(offset / HISTORY_PAGE_SIZE)) * HISTORY_PAGE_SIZE;
    this.set({ ...this.state, stage: "loading", offset, readError: undefined });
    try {
      let page = await this.actions.listHistory(offset, HISTORY_PAGE_SIZE);
      if (!current()) return;
      if (offset > 0 && offset >= page.total) {
        offset = Math.max(0, Math.ceil(page.total / HISTORY_PAGE_SIZE) - 1) * HISTORY_PAGE_SIZE;
        page = await this.actions.listHistory(offset, HISTORY_PAGE_SIZE);
        if (!current()) return;
      }
      if (this.state.page && page.revision < this.state.page.revision)
        throw new Error("History changed while loading. Refresh the list and try again.");
      this.set({ ...this.state, stage: "ready", page, offset });
    } catch (error) {
      if (current()) this.set({ ...this.state, stage: "failed", readError: String(error) });
    }
  }

  async handle(intent: HistoryIntent): Promise<void> {
    if (!this.connected || this.state.operation !== "idle") return;
    if (intent.type === "list-history") {
      this.set({ ...this.state, error: undefined });
      await this.load(intent.offset);
      return;
    }
    if (this.state.stage === "loading") return;
    const page = this.state.page;
    if (intent.type !== "sync-history" && (!page || this.state.stage !== "ready")) return;
    const entry = "id" in intent ? page?.entries.find((row) => row.id === intent.id) : undefined;
    if ("id" in intent && !entry) return;
    if (intent.type === "open-history" && !entry?.available) return;
    const lifecycle = this.lifecycle;
    const current = () => this.connected && lifecycle === this.lifecycle;
    this.set({ ...this.state, error: undefined, outcome: undefined, operation: "confirming" });
    let attempted = false;
    try {
      if (intent.type === "delete-history" || intent.type === "delete-all-history") {
        const all = intent.type === "delete-all-history";
        const approved = await this.actions.confirmAction(
          all
            ? `Delete all ${page!.total} saved sessions from their Agents, then remove their Lens copies? This includes every page, Agent and working directory. Sessions the Agent cannot delete remain in Lens. This cannot be undone.`
            : `Delete “${entry!.title}” from ${entry!.agent}, then remove its Lens copy? If the Agent cannot delete it, the session remains in Lens. This cannot be undone.`,
          all ? "Delete All Session History?" : "Delete Session History?",
        );
        if (!current() || !approved) return;
      }
      attempted = true;
      this.set({
        ...this.state,
        operation:
          intent.type === "sync-history"
            ? "synchronizing"
            : intent.type === "open-history"
              ? "opening"
              : "deleting",
      });
      if (intent.type === "open-history") await this.actions.openHistory(intent.id, page!.revision);
      else {
        const result =
          intent.type === "sync-history"
            ? await this.actions.syncHistory()
            : intent.type === "delete-history"
              ? await this.actions.deleteHistory(intent.id, page!.revision)
              : await this.actions.deleteAllHistory(page!.revision);
        if (current())
          this.set({
            ...this.state,
            outcome: { kind: intent.type === "sync-history" ? "sync" : "delete", result },
          });
      }
    } catch (error) {
      if (current()) this.set({ ...this.state, error: String(error) });
    } finally {
      if (current()) {
        this.set({ ...this.state, operation: "idle" });
        if (attempted) await this.load(this.state.offset);
      }
    }
  }
}
