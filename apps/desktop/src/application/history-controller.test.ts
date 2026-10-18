import { describe, expect, it, vi } from "vitest";
import type { ReactiveControllerHost } from "lit";
import type { HistoryPage, HistoryResult } from "ui/contracts/view-models";
import type { WebviewPort } from "./webview-port";
import { HistoryController } from "./history-controller";

const page = (revision = 7, total = 1): HistoryPage => ({
  revision,
  total,
  entries: total
    ? [
        {
          id: "opaque/source/session",
          title: "Saved session",
          agent: "ChatGPT Codex",
          cwd: "/other/project",
          updated_at: null,
          state: "local_only",
          available: true,
        },
      ]
    : [],
});
const result: HistoryResult = {
  succeeded: 2,
  failed: 1,
  purged: 3,
  errors: ["Agent unavailable; retained"],
};

function setup() {
  const host = {
    addController: vi.fn<ReactiveControllerHost["addController"]>(),
    requestUpdate: vi.fn<() => void>(),
    removeController: vi.fn<ReactiveControllerHost["removeController"]>(),
    updateComplete: Promise.resolve(true),
  };
  const port = {
    listHistory: vi.fn<WebviewPort["listHistory"]>().mockResolvedValue(page()),
    openHistory: vi.fn<WebviewPort["openHistory"]>().mockResolvedValue(),
    deleteHistory: vi.fn<WebviewPort["deleteHistory"]>().mockResolvedValue(result),
    deleteAllHistory: vi.fn<WebviewPort["deleteAllHistory"]>().mockResolvedValue(result),
    syncHistory: vi.fn<WebviewPort["syncHistory"]>().mockResolvedValue(result),
    confirmAction: vi.fn<WebviewPort["confirmAction"]>().mockResolvedValue(true),
  };
  const controller = new HistoryController(host, port);
  controller.hostConnected();
  return { controller, port, host };
}

describe("HistoryController", () => {
  it("reads the cache without Agent synchronization and admits only the latest page request", async () => {
    const { controller, port } = setup();
    let resolve!: (value: HistoryPage) => void;
    port.listHistory.mockReturnValueOnce(
      new Promise((next) => {
        resolve = next;
      }),
    );
    const first = controller.load(0);
    port.listHistory.mockResolvedValueOnce(page(9, 100));
    await controller.load(50);
    resolve(page(7, 100));
    await first;
    expect(controller.state.page?.revision).toBe(9);
    expect(controller.state.offset).toBe(50);
    expect(port.listHistory).toHaveBeenNthCalledWith(2, 50, 50);
    expect(port.syncHistory).not.toHaveBeenCalled();
  });

  it("rejects a lower catalog revision and preserves the admitted page", async () => {
    const { controller, port } = setup();
    await controller.load(0);
    port.listHistory.mockResolvedValueOnce(page(6));
    await controller.load(0);
    expect(controller.state.stage).toBe("failed");
    expect(controller.state.page?.revision).toBe(7);
    expect(controller.state.readError).toContain("History changed");
  });

  it("invalidates reads and operation completions across disconnect and reconnect", async () => {
    const { controller, port } = setup();
    let resolve!: (value: HistoryPage) => void;
    port.listHistory.mockReturnValueOnce(
      new Promise((next) => {
        resolve = next;
      }),
    );
    const read = controller.load(0);
    controller.hostDisconnected();
    controller.hostConnected();
    resolve(page());
    await read;
    expect(controller.state.stage).toBe("inactive");
    let finish!: (value: HistoryResult) => void;
    port.syncHistory.mockReturnValueOnce(
      new Promise((next) => {
        finish = next;
      }),
    );
    const sync = controller.handle({ type: "sync-history" });
    controller.hostDisconnected();
    controller.hostConnected();
    finish(result);
    await sync;
    expect(controller.state.stage).toBe("inactive");
    expect(controller.state.outcome).toBeUndefined();
    expect(port.listHistory).toHaveBeenCalledTimes(1);
  });

  it("captures opaque identity and revision before delete confirmation and locks competing actions", async () => {
    const { controller, port } = setup();
    await controller.load(0);
    let confirm!: (approved: boolean) => void;
    port.confirmAction.mockReturnValueOnce(
      new Promise((next) => {
        confirm = next;
      }),
    );
    const deletion = controller.handle({ type: "delete-history", id: "opaque/source/session" });
    expect(controller.state.operation).toBe("confirming");
    await controller.load(50);
    await controller.handle({ type: "sync-history" });
    expect(port.listHistory).toHaveBeenCalledTimes(1);
    expect(port.syncHistory).not.toHaveBeenCalled();
    confirm(true);
    await deletion;
    expect(port.deleteHistory).toHaveBeenCalledExactlyOnceWith("opaque/source/session", 7);
    expect(port.listHistory).toHaveBeenCalledTimes(2);
    expect(controller.state.outcome).toEqual({ kind: "delete", result });
  });

  it("cancels a destructive action without deletion or refresh", async () => {
    const { controller, port } = setup();
    await controller.load(0);
    port.confirmAction.mockResolvedValueOnce(false);
    await controller.handle({ type: "delete-all-history" });
    expect(port.confirmAction).toHaveBeenCalledWith(
      expect.stringContaining("every page, Agent and working directory"),
      "Delete All Session History?",
    );
    expect(port.deleteAllHistory).not.toHaveBeenCalled();
    expect(port.listHistory).toHaveBeenCalledTimes(1);
    expect(controller.state.operation).toBe("idle");
  });

  it("does not perform a confirmed deletion after the owner has disconnected", async () => {
    const { controller, port } = setup();
    await controller.load(0);
    let confirm!: (approved: boolean) => void;
    port.confirmAction.mockReturnValueOnce(
      new Promise((next) => {
        confirm = next;
      }),
    );
    const pending = controller.handle({ type: "delete-all-history" });
    controller.hostDisconnected();
    controller.hostConnected();
    confirm(true);
    await pending;
    expect(port.deleteAllHistory).not.toHaveBeenCalled();
  });

  it("synchronizes all DB targets independently of snapshot/selected Agent and retains partial results", async () => {
    const { controller, port } = setup();
    await controller.handle({ type: "sync-history" });
    expect(port.syncHistory).toHaveBeenCalledExactlyOnceWith();
    expect(controller.state.outcome).toEqual({ kind: "sync", result });
    expect(controller.state.stage).toBe("ready");
    expect(port.listHistory).toHaveBeenCalledExactlyOnceWith(0, 50);
  });

  it("refreshes after open and stale mutation rejection without hiding the failure", async () => {
    const { controller, port } = setup();
    await controller.load(0);
    port.listHistory.mockResolvedValueOnce(page(8));
    await controller.handle({ type: "open-history", id: "opaque/source/session" });
    expect(port.openHistory).toHaveBeenCalledExactlyOnceWith("opaque/source/session", 7);
    expect(controller.state.page?.revision).toBe(8);
    port.deleteHistory.mockRejectedValueOnce(new Error("Stale revision"));
    await controller.handle({ type: "delete-history", id: "opaque/source/session" });
    expect(port.deleteHistory).toHaveBeenCalledWith("opaque/source/session", 8);
    expect(controller.state.error).toContain("Stale revision");
    expect(controller.state.readError).toContain("History changed");
    expect(port.listHistory).toHaveBeenCalledTimes(3);
  });

  it("returns to the last existing page when deletions empty the current page", async () => {
    const { controller, port } = setup();
    port.listHistory.mockResolvedValueOnce(page(7, 51));
    await controller.load(50);
    port.listHistory.mockResolvedValueOnce(page(8, 1)).mockResolvedValueOnce(page(8, 1));
    await controller.handle({ type: "delete-all-history" });
    expect(port.deleteAllHistory).toHaveBeenCalledExactlyOnceWith(7);
    expect(port.listHistory).toHaveBeenLastCalledWith(0, 50);
    expect(controller.state.offset).toBe(0);
    expect(controller.state.page?.total).toBe(1);
  });
});
