import { afterEach, expect, it, vi } from "vitest";
import { ConversationRenderCache } from "./conversation-render-cache";
class ControlledWorker {
  static current: ControlledWorker;
  onmessage?: (event: { data: unknown }) => void;
  onerror?: () => void;
  onmessageerror?: () => void;
  sent: Array<{ id: number }> = [];
  terminated = false;
  constructor() {
    ControlledWorker.current = this;
  }
  postMessage(value: { id: number }): void {
    this.sent.push(value);
  }
  terminate(): void {
    this.terminated = true;
  }
  emit(data: unknown): void {
    this.onmessage?.({ data });
  }
}
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});
it("sends no work before ready then resolves the matching worker response", async () => {
  vi.stubGlobal("Worker", ControlledWorker);
  const cache = new ConversationRenderCache();
  const result = cache.resolve("a", { type: "html", text: "hello" });
  const worker = ControlledWorker.current;
  expect(worker.sent).toHaveLength(0);
  worker.emit({ type: "ready" });
  await Promise.resolve();
  expect(worker.sent).toHaveLength(1);
  worker.emit({ id: worker.sent[0]!.id, value: { document: "<p>hello</p>", notices: [] } });
  expect((await result).html?.document).toBe("<p>hello</p>");
  cache.clear();
});
it("bounds silent startup and preserves its failure for queued jobs", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("Worker", ControlledWorker);
  const cache = new ConversationRenderCache();
  const result = cache
    .resolve("a", { type: "html", text: "hello" })
    .catch((error: unknown) => error);
  const worker = ControlledWorker.current;
  await vi.advanceTimersByTimeAsync(10000);
  expect(String(await result)).toContain("could not start preparing");
  expect(worker.terminated).toBe(true);
  await expect(cache.resolve("b", { type: "html", text: "other" })).rejects.toThrow(
    "could not start preparing",
  );
});
it("bounds processing after a successful startup", async () => {
  vi.useFakeTimers();
  vi.stubGlobal("Worker", ControlledWorker);
  const cache = new ConversationRenderCache();
  const result = cache
    .resolve("a", { type: "html", text: "hello" })
    .catch((error: unknown) => error);
  ControlledWorker.current.emit({ type: "ready" });
  await vi.advanceTimersByTimeAsync(30000);
  expect(String(await result)).toContain("took too long to prepare");
  expect(ControlledWorker.current.terminated).toBe(true);
});
