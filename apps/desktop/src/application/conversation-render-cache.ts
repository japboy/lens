import type { DocumentBlock } from "./session-document";
import type { PreparedHtmlPreview } from "../html-output";

export type DeferredBlock = Extract<DocumentBlock, { type: "deferred" }>;
export type BlockLoader = (block: DeferredBlock) => Promise<DocumentBlock>;
export interface PreparedConversationBlock {
  block: DocumentBlock;
  html?: PreparedHtmlPreview;
}
export const CONVERSATION_CACHE_BYTES = 8 * 1024 * 1024;
const MAX_CONCURRENT_LOADS = 4;

/** Byte-bounded LRU of visible content and prepared HTML, never a full transcript copy. */
export class ConversationRenderCache {
  private values = new Map<string, { value: PreparedConversationBlock; bytes: number }>();
  private pending = new Map<string, Promise<PreparedConversationBlock>>();
  private bytes = 0;
  private active = 0;
  private queue: Array<() => void> = [];
  private generation = 0;
  private queuedCancels = new Set<() => void>();
  private worker: Worker | undefined;
  private workerSequence = 0;
  private workerReady: Promise<void> | undefined;
  private rejectWorkerReady: ((error: Error) => void) | undefined;
  private workerStartupTimer: ReturnType<typeof setTimeout> | undefined;
  private workerFailure: Error | undefined;
  private workerRequests = new Map<
    number,
    { resolve: (value: PreparedHtmlPreview) => void; reject: (error: Error) => void }
  >();
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
      if (body.type === "html") value.html = await this.prepare(body.text);
      if (signal?.aborted || generation !== this.generation)
        throw new Error("Conversation content superseded");
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
    this.failWorker(new Error("Conversation preparation superseded"));
    this.workerFailure = undefined;
  }
  private failWorker(error: Error): void {
    this.workerFailure = error;
    clearTimeout(this.workerStartupTimer);
    this.workerStartupTimer = undefined;
    this.rejectWorkerReady?.(error);
    this.rejectWorkerReady = undefined;
    this.workerReady = undefined;
    this.worker?.terminate();
    this.worker = undefined;
    for (const request of this.workerRequests.values()) request.reject(error);
    this.workerRequests.clear();
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
          reject(new Error("Conversation preparation capacity exceeded"));
          return;
        }
        this.queue.push(start);
        this.queuedCancels.add(cancel);
        signal?.addEventListener("abort", cancel, { once: true });
      }
    });
  }
  private async prepare(text: string): Promise<PreparedHtmlPreview> {
    if (typeof Worker === "undefined") {
      // Non-browser test hosts; production WebViews use the worker path.
      return import("../html-output").then(({ prepareHtmlPreview }) => prepareHtmlPreview(text));
    }
    if (this.workerFailure) throw this.workerFailure;
    if (!this.worker) {
      const worker = new Worker(new URL("./conversation-html.worker.ts", import.meta.url), {
        type: "module",
      });
      this.worker = worker;
      this.workerReady = new Promise<void>((resolve, reject) => {
        this.rejectWorkerReady = reject;
        this.workerStartupTimer = setTimeout(() => {
          if (this.worker === worker)
            this.failWorker(
              new Error(
                "Conversation content could not start preparing. Reopen the tab to try again.",
              ),
            );
        }, 10_000);
        worker.onmessage = (
          event: MessageEvent<{
            type?: "ready";
            id?: number;
            value?: PreparedHtmlPreview;
            error?: string;
          }>,
        ) => {
          if (this.worker !== worker) return;
          if (event.data.type === "ready") {
            clearTimeout(this.workerStartupTimer);
            this.workerStartupTimer = undefined;
            this.rejectWorkerReady = undefined;
            resolve();
            return;
          }
          if (event.data.id === undefined) return;
          const request = this.workerRequests.get(event.data.id);
          if (!request) return;
          this.workerRequests.delete(event.data.id);
          if (event.data.value !== undefined) request.resolve(event.data.value);
          else request.reject(new Error(event.data.error ?? "Conversation preparation failed"));
        };
        worker.onerror = () => {
          if (this.worker === worker)
            this.failWorker(new Error("Conversation worker failed during startup or processing"));
        };
        worker.onmessageerror = () => {
          if (this.worker === worker)
            this.failWorker(new Error("Conversation worker response could not be decoded"));
        };
      });
    }
    const worker = this.worker;
    await this.workerReady;
    if (this.worker !== worker) throw new Error("Conversation preparation superseded");
    const id = ++this.workerSequence;
    return new Promise((resolve, reject) => {
      const deadline = setTimeout(() => {
        if (this.worker === worker)
          this.failWorker(
            new Error(
              "Conversation content took too long to prepare. Reopen the tab to try again.",
            ),
          );
      }, 30_000);
      this.workerRequests.set(id, {
        resolve: (value) => {
          clearTimeout(deadline);
          resolve(value);
        },
        reject: (error) => {
          clearTimeout(deadline);
          reject(error);
        },
      });
      try {
        worker.postMessage({ id, text });
      } catch (error) {
        this.failWorker(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }
}
