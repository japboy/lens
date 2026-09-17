import type { DocumentBlock } from "./session-document";

export type DeferredBlock = Extract<DocumentBlock, { type: "deferred" }>;
export type BlockLoader = (block: DeferredBlock) => Promise<DocumentBlock>;
export interface PreparedConversationBlock {
  block: DocumentBlock;
}
export const CONVERSATION_CACHE_BYTES = 8 * 1024 * 1024;
const MAX_CONCURRENT_LOADS = 4;

/** Byte-bounded LRU of visible block bodies, never a full transcript copy. */
export class ConversationRenderCache {
  private values = new Map<string, { value: PreparedConversationBlock; bytes: number }>();
  private pending = new Map<string, Promise<PreparedConversationBlock>>();
  private bytes = 0;
  private active = 0;
  private queue: Array<() => void> = [];
  private generation = 0;
  private queuedCancels = new Set<() => void>();
  constructor(readonly budget = CONVERSATION_CACHE_BYTES) {}
  get queuedJobs(): number {
    return this.queue.length;
  }
  get retainedBytes(): number {
    return this.bytes;
  }
  get retainedEntries(): number {
    return this.values.size;
  }
  peek(key: string): PreparedConversationBlock | undefined {
    const found = this.values.get(key);
    if (!found) return undefined;
    this.values.delete(key);
    this.values.set(key, found);
    return found.value;
  }
  async resolve(
    key: string,
    block: DocumentBlock,
    loader?: BlockLoader,
    signal?: AbortSignal,
  ): Promise<PreparedConversationBlock> {
    const cached = this.peek(key);
    if (cached) return cached;
    const existing = this.pending.get(key);
    if (existing) return existing;
    const generation = this.generation;
    const task = this.schedule(async () => {
      if (signal?.aborted || generation !== this.generation)
        throw new Error("Conversation content superseded");
      const body =
        block.type === "deferred"
          ? await (loader
              ? loader(block)
              : Promise.reject(new Error("Conversation body loader unavailable")))
          : block;
      if (signal?.aborted || generation !== this.generation)
        throw new Error("Conversation content superseded");
      if (body.type === "deferred") throw new Error("Conversation body was not resolved");
      const value: PreparedConversationBlock = { block: body };
      const bytes = new TextEncoder().encode(JSON.stringify(value)).byteLength;
      if (bytes <= this.budget) {
        while (this.bytes + bytes > this.budget && this.values.size) {
          const oldest = this.values.keys().next().value!;
          this.bytes -= this.values.get(oldest)!.bytes;
          this.values.delete(oldest);
        }
        this.values.set(key, { value, bytes });
        this.bytes += bytes;
      }
      return value;
    }, signal);
    this.pending.set(key, task);
    const forget = () => {
      if (this.pending.get(key) === task) this.pending.delete(key);
    };
    signal?.addEventListener("abort", forget, { once: true });
    try {
      return await task;
    } finally {
      signal?.removeEventListener("abort", forget);
      if (this.pending.get(key) === task) this.pending.delete(key);
    }
  }
  clear(): void {
    this.values.clear();
    this.bytes = 0;
    this.suspend();
  }
  suspend(): void {
    this.generation++;
    this.pending.clear();
    for (const cancel of this.queuedCancels) cancel();
  }

  private schedule<T>(run: () => Promise<T>, signal?: AbortSignal): Promise<T> {
    return new Promise((resolve, reject) => {
      const cancel = () => {
        this.queuedCancels.delete(cancel);
        signal?.removeEventListener("abort", cancel);
        const index = this.queue.indexOf(start);
        if (index >= 0) this.queue.splice(index, 1);
        reject(new Error("Conversation content superseded"));
      };
      const start = () => {
        this.queuedCancels.delete(cancel);
        signal?.removeEventListener("abort", cancel);
        if (signal?.aborted) {
          cancel();
          return;
        }
        this.active++;
        void run()
          .then(resolve, reject)
          .finally(() => {
            this.active--;
            this.queue.shift()?.();
          });
      };
      if (signal?.aborted) {
        cancel();
        return;
      }
      if (this.active < MAX_CONCURRENT_LOADS) start();
      else {
        if (this.queue.length >= 512) {
          reject(new Error("Conversation body loading capacity exceeded"));
          return;
        }
        this.queue.push(start);
        this.queuedCancels.add(cancel);
        signal?.addEventListener("abort", cancel, { once: true });
      }
    });
  }
}
