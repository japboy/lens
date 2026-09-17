import { describe, expect, it, vi } from "vitest";
import type { ReactiveControllerHost } from "lit";
import type { WebviewPort } from "./webview-port";
import type { SessionView } from "./session-document";
import { SessionViewController } from "./session-view-controller";

describe("session view delivery", () => {
  it("retains a newer event when the initial response arrives late, and ignores events after detach", async () => {
    let listener!: (view: SessionView) => void;
    let resolve!: (view: SessionView) => void;
    const unlisten = vi.fn<() => void>();
    const host = {
      addController: vi.fn<ReactiveControllerHost["addController"]>(),
      requestUpdate: vi.fn<ReactiveControllerHost["requestUpdate"]>(),
    } as unknown as ReactiveControllerHost;
    const port = {
      subscribeToSessionView: vi.fn<WebviewPort["subscribeToSessionView"]>(
        async (callback: typeof listener) => {
          listener = callback;
          return unlisten;
        },
      ),
      getSessionView: vi.fn<WebviewPort["getSessionView"]>(
        () =>
          new Promise<SessionView>((r) => {
            resolve = r;
          }),
      ),
    } as unknown as WebviewPort;
    const controller = new SessionViewController(host, port);
    controller.hostConnected();
    await vi.waitFor(() => expect(resolve).toBeDefined());
    listener({ revision: 3, phase: "ready", session_id: "new" });
    resolve({ revision: 2, phase: "loading", session_id: "old" });
    await Promise.resolve();
    expect(controller.view?.session_id).toBe("new");
    controller.hostDisconnected();
    listener({ revision: 4, phase: "ready", session_id: "stale" });
    expect(controller.view?.session_id).toBe("new");
    expect(unlisten).toHaveBeenCalledOnce();
  });
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function harness() {
  const listeners: Array<(view: SessionView) => void> = [];
  const port = {
    subscribeToSessionView: vi.fn<WebviewPort["subscribeToSessionView"]>(async (callback) => {
      listeners.push(callback);
      return () => {};
    }),
    getSessionView: vi.fn<WebviewPort["getSessionView"]>(),
    getSessionBlock: vi.fn<WebviewPort["getSessionBlock"]>(),
  };
  const host = {
    addController: vi.fn<ReactiveControllerHost["addController"]>(),
    requestUpdate: vi.fn<ReactiveControllerHost["requestUpdate"]>(),
  } as unknown as ReactiveControllerHost;
  const controller = new SessionViewController(host, port as unknown as WebviewPort);
  return { controller, port, listeners, send: (view: SessionView) => listeners.at(-1)!(view) };
}
const ref = (revision: number, byte_length: number, append_only = true) => ({
  type: "deferred" as const,
  entry_id: "entry",
  block_index: 0,
  content_type: "markdown" as const,
  revision,
  byte_length,
  append_only,
});
const viewWith = (
  revision: number,
  reference = ref(revision, 3),
  generation = "first",
): SessionView => ({
  revision,
  generation,
  phase: "live",
  conversation: {
    entries: [{ id: "entry", kind: "message", role: "assistant", blocks: [reference] }],
  },
});

it("resynchronizes a missing patch base without applying the gap", async () => {
  const h = harness();
  const next = deferred<SessionView>();
  h.port.getSessionView.mockResolvedValueOnce(viewWith(1)).mockReturnValueOnce(next.promise);
  h.controller.hostConnected();
  await vi.waitFor(() => expect(h.controller.view?.revision).toBe(1));
  h.send({
    revision: 3,
    generation: "first",
    phase: "live",
    patch: { base_revision: 2, index: 0, entry: viewWith(3).conversation!.entries[0]! },
  });
  expect(h.controller.view?.revision).toBe(1);
  expect(h.port.getSessionView).toHaveBeenCalledTimes(2);
  next.resolve(viewWith(3));
  await vi.waitFor(() => expect(h.controller.view?.revision).toBe(3));
});

it("resynchronizes a reconnected host while its old request is still pending", async () => {
  const h = harness();
  const old = deferred<SessionView>();
  const current = deferred<SessionView>();
  const gap = deferred<SessionView>();
  h.port.getSessionView
    .mockReturnValueOnce(old.promise)
    .mockReturnValueOnce(current.promise)
    .mockReturnValueOnce(gap.promise);
  h.controller.hostConnected();
  await vi.waitFor(() => expect(h.port.getSessionView).toHaveBeenCalledTimes(1));
  h.controller.hostDisconnected();
  h.controller.hostConnected();
  await vi.waitFor(() => expect(h.port.getSessionView).toHaveBeenCalledTimes(2));
  old.resolve(viewWith(1));
  await Promise.resolve();
  h.send({
    revision: 3,
    generation: "first",
    phase: "live",
    patch: { base_revision: 2, index: 0, entry: viewWith(3).conversation!.entries[0]! },
  });
  expect(h.port.getSessionView).toHaveBeenCalledTimes(2);
  current.resolve(viewWith(2));
  await vi.waitFor(() => expect(h.port.getSessionView).toHaveBeenCalledTimes(3));
  gap.resolve(viewWith(3));
  await vi.waitFor(() => expect(h.controller.view?.revision).toBe(3));
});

it("rejects a body returned after the selected session changes", async () => {
  const h = harness();
  const body = deferred<Awaited<ReturnType<WebviewPort["getSessionBlock"]>>>();
  h.port.getSessionView.mockResolvedValue(viewWith(1));
  h.port.getSessionBlock.mockReturnValue(body.promise);
  h.controller.hostConnected();
  await vi.waitFor(() => expect(h.controller.view).toBeDefined());
  const pending = h.controller.loadBlock(ref(1, 3)).catch((error: unknown) => error);
  h.send(viewWith(2, ref(2, 3), "second"));
  body.resolve({
    generation: "first",
    revision: 1,
    offset: 0,
    block: { type: "markdown", text: "old" },
  });
  expect(await pending).toEqual(
    expect.objectContaining({ message: "Session content response is stale" }),
  );
});

it("requests append suffixes using UTF-8 byte offsets and retains identical chunks", async () => {
  const h = harness();
  h.port.getSessionView.mockResolvedValue(viewWith(1));
  h.port.getSessionBlock
    .mockResolvedValueOnce({
      generation: "first",
      revision: 1,
      offset: 0,
      block: { type: "markdown", text: "猫" },
    })
    .mockResolvedValueOnce({
      generation: "first",
      revision: 2,
      offset: 3,
      block: { type: "markdown", text: "猫" },
    });
  h.controller.hostConnected();
  await vi.waitFor(() => expect(h.controller.view).toBeDefined());
  expect(await h.controller.loadBlock(ref(1, 3))).toEqual({ type: "markdown", text: "猫" });
  h.send(viewWith(2, ref(2, 6)));
  expect(await h.controller.loadBlock(ref(2, 6))).toEqual({ type: "markdown", text: "猫猫" });
  expect(h.port.getSessionBlock.mock.calls[1]![0].offset).toBe(3);
});

it("fetches replacement tool bodies in full instead of appending old text", async () => {
  const h = harness();
  const toolView = (revision: number) => ({
    ...viewWith(revision, ref(revision, 3, false)),
    conversation: {
      entries: [
        {
          id: "entry",
          kind: "tool" as const,
          title: "Tool",
          status: "completed" as const,
          blocks: [ref(revision, 3, false)],
        },
      ],
    },
  });
  h.port.getSessionView.mockResolvedValue(toolView(1));
  h.port.getSessionBlock
    .mockResolvedValueOnce({
      generation: "first",
      revision: 1,
      offset: 0,
      block: { type: "markdown", text: "old" },
    })
    .mockResolvedValueOnce({
      generation: "first",
      revision: 2,
      offset: 0,
      block: { type: "markdown", text: "new" },
    });
  h.controller.hostConnected();
  await vi.waitFor(() => expect(h.controller.view).toBeDefined());
  await h.controller.loadBlock(ref(1, 3, false));
  h.send(toolView(2));
  expect(await h.controller.loadBlock(ref(2, 3, false))).toEqual({ type: "markdown", text: "new" });
  expect(h.port.getSessionBlock.mock.calls.map((call) => call[0].offset)).toEqual([0, 0]);
});
