import { expect, it, vi } from "vitest";
import type { EventCallback, Options } from "@tauri-apps/api/event";
import type { AppSnapshot } from "../types";

const currentWindow = vi.hoisted(() => ({ label: "lens-overlay" }));
const listen = vi.hoisted(() =>
  vi.fn<
    (event: string, handler: EventCallback<AppSnapshot>, options?: Options) => Promise<() => void>
  >(),
);
vi.mock("@tauri-apps/api/event", () => ({ listen }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => currentWindow }));

import { tauriWebviewPort } from "./webview-port";

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
