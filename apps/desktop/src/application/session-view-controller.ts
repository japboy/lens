import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { DeferredDocumentBlock, DocumentBlock, SessionView } from "./session-document";
import type { Unlisten, WebviewPort } from "./webview-port";

const TEXT_CACHE_BYTES = 8 * 1024 * 1024;

/** Session metadata is ordered independently from lazily fetched visible bodies. */
export class SessionViewController implements ReactiveController {
  view: SessionView | undefined;
  private generation = 0;
  private unlisten: Unlisten | undefined;
  private synchronization: { generation: number; requested: boolean } | undefined;
  private textCache = new Map<string, { text: string; revision: number; bytes: number }>();
  private textCacheBytes = 0;
  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly port: WebviewPort,
  ) {
    host.addController(this);
  }
  hostConnected(): void {
    void this.load(++this.generation);
  }
  hostDisconnected(): void {
    ++this.generation;
    this.unlisten?.();
    this.unlisten = undefined;
    this.textCache.clear();
    this.textCacheBytes = 0;
  }
  private apply(view: SessionView, generation: number): void {
    if (generation !== this.generation || (this.view && view.revision <= this.view.revision))
      return;
    if (view.patch) {
      const current = this.view;
      if (
        !current ||
        current.generation !== view.generation ||
        current.revision !== view.patch.base_revision ||
        !current.conversation ||
        view.patch.index > current.conversation.entries.length
      ) {
        void this.resync(generation);
        return;
      }
      const entries = current.conversation.entries.slice();
      entries[view.patch.index] = view.patch.entry;
      view = { ...view, conversation: { entries } };
    }
    if (this.view?.generation !== view.generation) {
      this.textCache.clear();
      this.textCacheBytes = 0;
    }
    this.view = view;
    this.host.requestUpdate();
  }
  private async resync(generation: number): Promise<void> {
    if (generation !== this.generation) return;
    if (this.synchronization?.generation === generation) {
      this.synchronization.requested = true;
      return;
    }
    const synchronization = { generation, requested: false };
    this.synchronization = synchronization;
    try {
      do {
        synchronization.requested = false;
        const view = await this.port.getSessionView();
        if (generation !== this.generation) return;
        this.apply(view, generation);
      } while (synchronization.requested && generation === this.generation);
    } catch (error) {
      if (generation === this.generation) {
        this.view = this.view
          ? { ...this.view, error: `Unable to synchronize session: ${String(error)}` }
          : { revision: -1, phase: "failed", error: `Unable to load session: ${String(error)}` };
        this.host.requestUpdate();
      }
    } finally {
      if (this.synchronization === synchronization) this.synchronization = undefined;
    }
  }
  private async load(generation: number): Promise<void> {
    try {
      const unlisten = await this.port.subscribeToSessionView((view) =>
        this.apply(view, generation),
      );
      if (generation !== this.generation) {
        unlisten();
        return;
      }
      this.unlisten = unlisten;
      await this.resync(generation);
    } catch (error) {
      if (generation === this.generation && !this.view) {
        this.view = {
          revision: -1,
          phase: "failed",
          error: `Unable to load session: ${String(error)}`,
        };
        this.host.requestUpdate();
      }
    }
  }
  readonly loadBlock = async (reference: DeferredDocumentBlock): Promise<DocumentBlock> => {
    const generation = this.view?.generation;
    const connection = this.generation;
    if (!generation) throw new Error("Session content generation unavailable");
    const key = `${generation}:${reference.entry_id}:${reference.block_index}`;
    const previous = reference.append_only ? this.textCache.get(key) : undefined;
    if (previous?.revision === reference.revision) return { type: "markdown", text: previous.text };
    const offset = previous && previous.bytes <= reference.byte_length ? previous.bytes : 0;
    const response = await this.port.getSessionBlock({
      generation,
      entry_id: reference.entry_id,
      block_index: reference.block_index,
      revision: reference.revision,
      offset,
    });
    if (
      connection !== this.generation ||
      this.view?.generation !== generation ||
      response.generation !== generation ||
      response.revision !== reference.revision
    )
      throw new Error("Session content response is stale");
    const current = this.view.conversation?.entries.find((entry) => entry.id === reference.entry_id)
      ?.blocks[reference.block_index];
    if (current?.type !== "deferred" || current.revision !== reference.revision)
      throw new Error("Session content revision changed");
    let block = response.block;
    if (block.type === "markdown" && response.offset > 0) {
      if (!previous || response.offset !== previous.bytes)
        throw new Error("Session content append base unavailable");
      block = { type: "markdown", text: previous.text + block.text };
    }
    if (
      block.type === "markdown" &&
      reference.append_only &&
      reference.byte_length <= TEXT_CACHE_BYTES
    ) {
      const old = this.textCache.get(key);
      if (old) {
        this.textCacheBytes -= old.bytes;
        this.textCache.delete(key);
      }
      this.textCache.set(key, {
        text: block.text,
        revision: reference.revision,
        bytes: reference.byte_length,
      });
      this.textCacheBytes += reference.byte_length;
      while (this.textCacheBytes > TEXT_CACHE_BYTES) {
        const oldest = this.textCache.keys().next().value!;
        this.textCacheBytes -= this.textCache.get(oldest)!.bytes;
        this.textCache.delete(oldest);
      }
    }
    return block;
  };
}
