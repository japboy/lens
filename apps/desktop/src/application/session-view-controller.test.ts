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
