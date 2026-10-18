import { expect, it, vi } from "vitest";
import type { EventCallback, Options } from "@tauri-apps/api/event";
import type { AppSnapshot } from "../types";

const currentWindow = vi.hoisted(() => ({ label: "lens-overlay" }));
const invoke = vi.hoisted(() => vi.fn<typeof import("@tauri-apps/api/core").invoke>());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const listen = vi.hoisted(() =>
  vi.fn<
    (event: string, handler: EventCallback<AppSnapshot>, options?: Options) => Promise<() => void>
  >(),
);
vi.mock("@tauri-apps/api/event", () => ({ listen }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => currentWindow }));

import { tauriWebviewPort } from "./webview-port";

it("passes opaque history identity and admitted revision to native history operations", async () => {
  const page = { revision: 42, total: 1, entries: [] };
  const result = { succeeded: 1, failed: 0, purged: 0, errors: [] };
  invoke
    .mockReset()
    .mockResolvedValueOnce(page)
    .mockResolvedValueOnce(undefined)
    .mockResolvedValueOnce(result)
    .mockResolvedValueOnce(result)
    .mockResolvedValueOnce(result);
  expect(await tauriWebviewPort.listHistory(50, 50)).toBe(page);
  expect(invoke).toHaveBeenNthCalledWith(1, "list_history", { offset: 50, limit: 50 });
  await tauriWebviewPort.openHistory("opaque:source/session", 42);
  expect(invoke).toHaveBeenNthCalledWith(2, "open_history", {
    id: "opaque:source/session",
    revision: 42,
  });
  expect(await tauriWebviewPort.deleteHistory("opaque:source/session", 42)).toBe(result);
  expect(invoke).toHaveBeenNthCalledWith(3, "delete_history", {
    id: "opaque:source/session",
    revision: 42,
  });
  expect(await tauriWebviewPort.deleteAllHistory(42)).toBe(result);
  expect(invoke).toHaveBeenNthCalledWith(4, "delete_all_history", { revision: 42 });
  expect(await tauriWebviewPort.syncHistory()).toBe(result);
  expect(invoke).toHaveBeenNthCalledWith(5, "sync_history");
});

it.each(["lens-overlay", "settings"])(
  "scopes snapshot subscriptions to the current %s WebviewWindow",
  async (label) => {
    currentWindow.label = label;
    const unlisten = vi.fn<() => void>();
    listen.mockReset().mockResolvedValue(unlisten);
    const receive = vi.fn<(snapshot: AppSnapshot) => void>();

    const unsubscribe = await tauriWebviewPort.subscribeToAppSnapshot(receive);
    expect(listen).toHaveBeenCalledWith("window-app-state-changed", expect.any(Function), {
      target: { kind: "WebviewWindow", label },
    });
    const payload = { revision: 17 } as AppSnapshot;
    listen.mock.calls[0]![1]({ event: "window-app-state-changed", id: 1, payload });
    expect(receive).toHaveBeenCalledExactlyOnceWith(payload);
    expect(unsubscribe).toBe(unlisten);
    unsubscribe();
    expect(unlisten).toHaveBeenCalledOnce();
  },
);
