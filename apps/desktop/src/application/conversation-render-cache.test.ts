import { describe, expect, it, vi } from "vitest";
import {
  ConversationRenderCache,
  type BlockLoader,
  type DeferredBlock,
} from "./conversation-render-cache";
const deferred: DeferredBlock = {
  type: "deferred",
  entry_id: "a",
  block_index: 0,
  content_type: "unsupported",
  revision: 1,
  byte_length: 100,
};
describe("visible conversation body cache", () => {
  it("returns HTML source unchanged without creating a worker or prepared preview", async () => {
    const worker = vi.fn<() => never>(() => {
      throw new Error("Unexpected worker");
    });
    vi.stubGlobal("Worker", worker);
    try {
      const cache = new ConversationRenderCache();
      const body = { type: "html" as const, text: '<script>throw "inert"</script><h1>Source</h1>' };
      const value = await cache.resolve("html", body);
      expect(value).toEqual({ block: body });
      expect(value.block).toBe(body);
      expect(worker).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });
  it("deduplicates visible requests and evicts within its byte budget", async () => {
    const cache = new ConversationRenderCache(100);
    const loader = vi.fn<BlockLoader>(async () => ({
      type: "unsupported" as const,
      content_type: "x".repeat(30),
    }));
    await Promise.all([cache.resolve("a", deferred, loader), cache.resolve("a", deferred, loader)]);
    expect(loader).toHaveBeenCalledTimes(1);
    await cache.resolve("b", deferred, loader);
    expect(cache.retainedBytes).toBeLessThanOrEqual(100);
    expect(cache.peek("a")).toBeUndefined();
    expect(cache.peek("b")).toBeDefined();
  });
  it("discards stale session responses while allowing a fresh request", async () => {
    const cache = new ConversationRenderCache();
    let finish!: (value: { type: "unsupported"; content_type: string }) => void;
    const old = cache.resolve(
      "a",
      deferred,
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const rejected = old.catch((error: unknown) => error);
    cache.clear();
    finish({ type: "unsupported", content_type: "old" });
    expect(String(await rejected)).toContain("superseded");
    expect(cache.retainedEntries).toBe(0);
    const value = await cache.resolve("a", deferred, async () => ({
      type: "unsupported",
      content_type: "new",
    }));
    expect(value.block).toEqual({ type: "unsupported", content_type: "new" });
  });
  it("removes offscreen or superseded queued bodies before loading", async () => {
    const cache = new ConversationRenderCache();
    const finishes: Array<(value: { type: "unsupported"; content_type: string }) => void> = [];
    const active = Array.from({ length: 4 }, (_, index) =>
      cache.resolve(
        String(index),
        deferred,
        () => new Promise((resolve) => finishes.push(resolve)),
      ),
    );
    const abort = new AbortController();
    const loader = vi.fn<BlockLoader>(async () => ({
      type: "unsupported" as const,
      content_type: "obsolete",
    }));
    const queued = cache.resolve("queued", deferred, loader, abort.signal);
    const rejected = queued.catch((error: unknown) => error);
    expect(cache.queuedJobs).toBe(1);
    abort.abort();
    expect(String(await rejected)).toContain("superseded");
    expect(cache.queuedJobs).toBe(0);
    finishes.forEach((finish) => finish({ type: "unsupported", content_type: "done" }));
    await Promise.all(active);
    expect(loader).not.toHaveBeenCalled();
    expect(
      (
        await cache.resolve("queued", deferred, async () => ({
          type: "unsupported",
          content_type: "fresh",
        }))
      ).block,
    ).toEqual({ type: "unsupported", content_type: "fresh" });
  });
  it("returns an oversized body intact without retaining it", async () => {
    const cache = new ConversationRenderCache(1);
    const body = { type: "unsupported" as const, content_type: "large".repeat(100) };
    expect((await cache.resolve("a", body)).block).toEqual(body);
    expect(cache.retainedBytes).toBe(0);
  });
});
