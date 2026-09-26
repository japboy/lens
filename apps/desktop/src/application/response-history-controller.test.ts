import type { DeferredDocumentBlock, DocumentBlock, SessionView } from "./session-document";
import { describe, expect, it, vi } from "vitest";
import type { WebviewPort } from "./webview-port";
import type { ReactiveControllerHost } from "lit";
import type { LensOutputBlock, LensState, LensResponseManifest } from "../types";
import {
  ResponseHistoryController,
  responseBlockIdentity,
  RESPONSE_BODY_CACHE_BYTES,
} from "./response-history-controller";

function response(id: string, sequence: number): LensResponseManifest {
  return {
    representation_id: id,
    sequence,
    run_id: id,
    prompt_execution_revision: 1,
    context_id: "ctx",
    context_revision: sequence,
    projection: { revision: 1, digest: "digest" },
    retained_bytes: 3,
    block_count: 3,
    blocks: [
      { type: "markdown", block_index: 0, byte_length: 3 },
      {
        type: "html",
        block_index: 1,
        resource_id: "shared",
        uri: "lens:html",
        mime_type: "text/html",
        byte_length: 3,
      },
      { type: "image", block_index: 2, mime_type: "image/png", byte_length: 1 },
    ],
  };
}
function lens(operation = "op", count = 1): LensState {
  return {
    operation_id: operation,
    stage: "completed",
    prompt_execution_revision: 1,
    output_blocks: [],
    response_history: {
      responses: Array.from({ length: count }, (_, i) => response(`r${i + 1}`, i + 1)),
      retained_bytes: count * 3,
      capacity_reached: false,
    },
  };
}
function setup() {
  const host = {
    addController: vi.fn<ReactiveControllerHost["addController"]>(),
    requestUpdate: vi.fn<ReactiveControllerHost["requestUpdate"]>(),
  } as unknown as ReactiveControllerHost;
  const port = {
    getResponseBlock: vi.fn<WebviewPort["getResponseBlock"]>(
      async (): Promise<LensOutputBlock> => ({
        type: "markdown",
        text: "abc",
      }),
    ),
    getHtmlOutput: vi.fn<WebviewPort["getHtmlOutput"]>(async () => "abc"),
  };
  return { controller: new ResponseHistoryController(host, port), port };
}
const deferred = <T>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
};

describe("committed response history bodies", () => {
  it("reconstructs all ordered manifests after missed events without fetching bodies or duplicating snapshots", () => {
    const { controller, port } = setup();
    controller.synchronize(lens());
    controller.synchronize(lens("op", 3));
    expect(controller.presentation?.responses.map((r) => r.sequence)).toEqual([1, 2, 3]);
    expect(controller.presentation?.media).toHaveLength(6);
    const presentation = controller.presentation;
    controller.synchronize(lens("op", 3));
    controller.synchronize(lens());
    expect(controller.presentation).toBe(presentation);
    expect(port.getResponseBlock).not.toHaveBeenCalled();
    expect(port.getHtmlOutput).not.toHaveBeenCalled();
    controller.hostDisconnected();
    controller.synchronize(lens("op", 3));
    expect(controller.presentation?.responses).toHaveLength(3);
  });
  it("loads old HTML by response scope, keeps IDs stable on append, and reloads evicted presentation demand from cache", async () => {
    const { controller, port } = setup();
    controller.synchronize(lens("op", 2));
    const first = responseBlockIdentity("op", "r1", 1),
      second = responseBlockIdentity("op", "r2", 1);
    controller.requestMedia([first]);
    await vi.waitFor(() =>
      expect(controller.presentation?.htmlContents.get(first)?.status).toBe("ready"),
    );
    controller.synchronize(lens("op", 3));
    expect(controller.presentation?.media[0]?.id).toBe(first);
    controller.requestMedia([second]);
    await vi.waitFor(() =>
      expect(controller.presentation?.htmlContents.get(second)?.status).toBe("ready"),
    );
    expect(controller.presentation?.htmlContents.has(first)).toBe(false);
    controller.requestMedia([first]);
    await vi.waitFor(() =>
      expect(controller.presentation?.htmlContents.get(first)?.status).toBe("ready"),
    );
    expect(port.getHtmlOutput.mock.calls).toEqual([
      ["op", "r1", "shared"],
      ["op", "r2", "shared"],
    ]);
  });
  it("retries a failed native media body only after explicit navigation requests it", async () => {
    const { controller, port } = setup();
    port.getHtmlOutput.mockRejectedValueOnce(new Error("temporarily unavailable"));
    controller.synchronize(lens());
    const id = responseBlockIdentity("op", "r1", 1);
    controller.requestMedia([id]);
    await vi.waitFor(() =>
      expect(controller.presentation?.htmlContents.get(id)?.status).toBe("failed"),
    );
    controller.requestMedia([id]);
    controller.synchronize(lens());
    expect(port.getHtmlOutput).toHaveBeenCalledTimes(1);
    await controller.retryMedia(id);
    expect(port.getHtmlOutput).toHaveBeenCalledTimes(2);
    expect(controller.presentation?.htmlContents.get(id)?.status).toBe("ready");
    expect(controller.presentation?.mediaErrors.has(id)).toBe(false);
    await controller.retryMedia(id);
    expect(port.getHtmlOutput).toHaveBeenCalledTimes(2);
  });
  it("deduplicates concurrent body requests and ignores stale operation results without blocking new demand", async () => {
    const { controller, port } = setup();
    const old = deferred<LensOutputBlock>();
    port.getResponseBlock.mockImplementationOnce(() => old.promise);
    controller.synchronize(lens());
    const first = controller.loadBlock("op", "r1", 0);
    const duplicate = controller.loadBlock("op", "r1", 0);
    const oldResult = Promise.allSettled([first, duplicate]);
    controller.synchronize(lens("new"));
    await expect(controller.loadBlock("new", "r1", 0)).resolves.toEqual({
      type: "markdown",
      text: "abc",
    });
    old.resolve({ type: "markdown", text: "abc" });
    expect((await oldResult).map((r) => r.status)).toEqual(["rejected", "rejected"]);
    expect(port.getResponseBlock).toHaveBeenCalledTimes(2);
    expect(controller.presentation?.scopeId).toBe("new");
  });
  it("bounds concurrent calls, rejects unexpected types/length, and exposes HTML failure without erasing history", async () => {
    const { controller, port } = setup();
    controller.synchronize(lens("op", 3));
    const pending = [
      deferred<LensOutputBlock>(),
      deferred<LensOutputBlock>(),
      deferred<LensOutputBlock>(),
    ];
    let calls = 0;
    port.getResponseBlock.mockImplementation(() => pending[calls++]!.promise);
    const results = [1, 2, 3].map((n) => controller.loadBlock("op", `r${n}`, 0));
    const settled = Promise.allSettled(results);
    expect(calls).toBe(2);
    pending[0]!.resolve({ type: "markdown", text: "abc" });
    await vi.waitFor(() => expect(calls).toBe(3));
    pending[1]!.resolve({ type: "markdown", text: "invalid length" });
    pending[2]!.resolve({ type: "unsupported", content_type: "audio" });
    expect((await settled).map((r) => r.status)).toEqual(["fulfilled", "rejected", "rejected"]);
    port.getHtmlOutput.mockRejectedValue(new Error("native denied"));
    const id = responseBlockIdentity("op", "r1", 1);
    controller.requestMedia([id]);
    await vi.waitFor(() =>
      expect(controller.presentation?.htmlContents.get(id)?.status).toBe("failed"),
    );
    expect(controller.presentation?.responses).toHaveLength(3);
  });
  it("evicts cached bodies within a fixed budget, while retaining authoritative manifests", async () => {
    const { controller, port } = setup();
    const state = lens("op", 3);
    const text = "x".repeat(Math.floor(RESPONSE_BODY_CACHE_BYTES / 2));
    for (const r of state.response_history!.responses)
      r.blocks[0] = { type: "markdown", block_index: 0, byte_length: text.length };
    port.getResponseBlock.mockResolvedValue({ type: "markdown", text });
    controller.synchronize(state);
    await controller.loadBlock("op", "r1", 0);
    await controller.loadBlock("op", "r2", 0);
    await controller.loadBlock("op", "r1", 0);
    expect(port.getResponseBlock).toHaveBeenCalledTimes(3);
    expect(controller.presentation?.responses).toHaveLength(3);
  });
  it("preserves history through pause/resume and capacity state, rejecting results after disconnect", async () => {
    const { controller, port } = setup();
    controller.synchronize(lens());
    const original = controller.presentation;
    controller.synchronize({
      ...lens(),
      live: {
        lifecycle: "paused",
        health: "healthy",
        freshness: "current",
        agent_refresh_interval_seconds: 30,
      },
    });
    expect(controller.presentation).toBe(original);
    const pending = deferred<string>();
    port.getHtmlOutput.mockReturnValue(pending.promise);
    const id = responseBlockIdentity("op", "r1", 1);
    controller.requestMedia([id]);
    controller.hostDisconnected();
    pending.resolve("abc");
    await Promise.resolve();
    await Promise.resolve();
    expect(controller.presentation).toBeUndefined();
    const full = lens();
    full.response_history!.capacity_reached = true;
    controller.synchronize(full);
    expect(controller.presentation?.capacityReached).toBe(true);
  });
  it("does not cache oversized valid text and keeps in-flight concurrency bounded across operation replacements", async () => {
    const { controller, port } = setup();
    const text = "x".repeat(RESPONSE_BODY_CACHE_BYTES + 1);
    const state = lens();
    state.response_history!.responses[0]!.blocks[0] = {
      type: "markdown",
      block_index: 0,
      byte_length: text.length,
    };
    controller.synchronize(state);
    port.getResponseBlock.mockResolvedValue({ type: "markdown", text });
    await expect(controller.loadBlock("op", "r1", 0)).resolves.toHaveProperty("text", text);
    await controller.loadBlock("op", "r1", 0);
    expect(port.getResponseBlock).toHaveBeenCalledTimes(2);
    const old = [deferred<LensOutputBlock>(), deferred<LensOutputBlock>()];
    let count = 0;
    port.getResponseBlock.mockImplementation(() => old[count++]!.promise);
    controller.synchronize(lens("old", 2));
    const settled = Promise.allSettled([
      controller.loadBlock("old", "r1", 0),
      controller.loadBlock("old", "r2", 0),
    ]);
    await vi.waitFor(() => expect(count).toBe(2));
    controller.synchronize(lens("new"));
    const current = controller.loadBlock("new", "r1", 0);
    expect(count).toBe(2);
    port.getResponseBlock.mockResolvedValue({ type: "markdown", text: "abc" });
    old[0]!.resolve({ type: "markdown", text: "abc" });
    await expect(current).resolves.toHaveProperty("text", "abc");
    old[1]!.resolve({ type: "markdown", text: "abc" });
    await settled;
  });
});

function replay(generation = "replay-one"): SessionView {
  return {
    phase: "ready",
    revision: 1,
    generation,
    interpretation: {
      responses: [1, 2, 3].map((sequence) => ({
        response_id: `response-${sequence}`,
        sequence,
        blocks: [
          {
            type: "markdown" as const,
            block_index: 0,
            byte_length: 3,
            source: {
              type: "deferred" as const,
              entry_id: `assistant-${sequence}`,
              block_index: 0,
              content_type: "markdown" as const,
              byte_length: 3,
              revision: 1,
            },
          },
          {
            type: "html" as const,
            block_index: 1,
            resource_id: `resource-${sequence}`,
            uri: `urn:history:${sequence}`,
            mime_type: "text/html" as const,
            byte_length: 3,
            source: {
              type: "deferred" as const,
              entry_id: `tool-${sequence}`,
              block_index: 2,
              content_type: "html" as const,
              byte_length: 3,
              revision: 1,
            },
          },
        ],
      })),
    },
  };
}
function setupReplay(
  loader = vi.fn<(reference: DeferredDocumentBlock) => Promise<DocumentBlock>>(async (source) =>
    source.content_type === "html"
      ? { type: "html", text: source.entry_id.endsWith("1") ? "old" : "new" }
      : { type: "markdown", text: "abc" },
  ),
) {
  const host = {
    addController: vi.fn<ReactiveControllerHost["addController"]>(),
    requestUpdate: vi.fn<ReactiveControllerHost["requestUpdate"]>(),
  } as unknown as ReactiveControllerHost;
  const port = {
    getResponseBlock: vi.fn<WebviewPort["getResponseBlock"]>(),
    getHtmlOutput: vi.fn<WebviewPort["getHtmlOutput"]>(),
  };
  return { controller: new ResponseHistoryController(host, port, loader), loader, port };
}

describe("restored session response history", () => {
  it("uses all native replay responses without live commit metadata or eager bodies", async () => {
    const { controller, loader, port } = setupReplay();
    controller.synchronizeHistory(replay());
    expect(controller.presentation?.scopeId).toBe("history:replay-one");
    expect(controller.presentation?.responses.map((response) => response.id)).toEqual([
      "response-1",
      "response-2",
      "response-3",
    ]);
    expect(controller.presentation?.media).toHaveLength(3);
    expect(loader).not.toHaveBeenCalled();
    expect(controller.presentation?.responses[0]).not.toHaveProperty("run_id");
    const id = responseBlockIdentity("history:replay-one", "response-1", 1);
    controller.requestMedia([id]);
    await vi.waitFor(() =>
      expect(controller.presentation?.htmlContents.get(id)).toMatchObject({
        content: "old",
        status: "ready",
      }),
    );
    expect(loader).toHaveBeenCalledWith(
      expect.objectContaining({ entry_id: "tool-1", block_index: 2 }),
    );
    await expect(controller.loadBlock("history:replay-one", "response-2", 0)).resolves.toEqual({
      type: "markdown",
      text: "abc",
    });
    expect(port.getResponseBlock).not.toHaveBeenCalled();
    expect(port.getHtmlOutput).not.toHaveBeenCalled();
    const next = responseBlockIdentity("history:replay-one", "response-3", 1);
    controller.requestMedia([next]);
    await vi.waitFor(() =>
      expect(controller.presentation?.htmlContents.get(next)?.status).toBe("ready"),
    );
    controller.requestMedia([id]);
    await vi.waitFor(() =>
      expect(controller.presentation?.htmlContents.get(id)).toMatchObject({ content: "old" }),
    );
    expect(loader).toHaveBeenCalledTimes(3);
  });
  it("rejects stale replay bodies after another session or live operation becomes authoritative", async () => {
    const pending = deferred<DocumentBlock>();
    const loader = vi.fn<(reference: DeferredDocumentBlock) => Promise<DocumentBlock>>(
      () => pending.promise,
    );
    const { controller, port } = setupReplay(loader);
    controller.synchronizeHistory(replay());
    const body = controller.loadBlock("history:replay-one", "response-1", 0);
    const outcome = Promise.allSettled([body]);
    controller.synchronizeHistory(replay("replay-two"));
    pending.resolve({ type: "markdown", text: "abc" });
    expect((await outcome)[0]?.status).toBe("rejected");
    expect(controller.presentation?.scopeId).toBe("history:replay-two");
    controller.synchronize(lens("live"));
    expect(controller.presentation?.scopeId).toBe("live");
    expect(controller.presentation?.responses.map((response) => response.id)).toEqual(["r1"]);
    expect(port.getResponseBlock).not.toHaveBeenCalled();
  });
});
