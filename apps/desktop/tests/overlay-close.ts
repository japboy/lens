// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSnapshot } from "../src/types";
import type { WebviewPort } from "../src/application/webview-port";
import type { LensOverlayView } from "../src/components/lens-overlay-view";
import { installGeneratedPage } from "../src/rendering/generated-page.test-helper";
import { startPage } from "../src/entries/start-page";

const port = vi.hoisted(() => ({
  subscribeToAppSnapshot: vi.fn<WebviewPort["subscribeToAppSnapshot"]>(),
  getAppSnapshot: vi.fn<WebviewPort["getAppSnapshot"]>(),
  closeCurrentWindow: vi.fn<WebviewPort["closeCurrentWindow"]>(),
  stopLens: vi.fn<WebviewPort["stopLens"]>(),
  pauseLens: vi.fn<WebviewPort["pauseLens"]>(),
}));
vi.mock("../src/application/webview-port", () => ({ tauriWebviewPort: port }));

const snapshot: AppSnapshot = {
  revision: 1,
  config: {
    agent: "codex",
    working_directory: "/tmp",
    agent_prompt_template: {
      schema_version: 1,
      common: "",
      full_projection: "",
      source_checkpoint: "",
      current_projection_retry: "",
    },
  },
  agent_selection: { stage: "selected", candidate: "codex", auth_methods: [] },
  agent_runtime: { stage: "ready", agent: "codex", downloaded_bytes: 0 },
  lens: { stage: "idle", output_blocks: [] },
};

beforeEach(() => {
  vi.resetAllMocks();
  window.history.replaceState({}, "", "/overlay.html?platform=macos");
  port.subscribeToAppSnapshot.mockResolvedValue(() => undefined);
  port.getAppSnapshot.mockResolvedValue(snapshot);
  port.closeCurrentWindow.mockResolvedValue(undefined);
  port.stopLens.mockResolvedValue(undefined);
});
afterEach(() => document.body.replaceChildren());

async function attach() {
  installGeneratedPage("overlay");
  const button = document
    .querySelector("lens-overlay-view")!
    .shadowRoot!.querySelector<HTMLButtonElement>(".close-button")!;
  expect(button.disabled).toBe(true);
  await startPage("overlay", () => import("../src/pages/overlay-page"));
  const view = document.querySelector<LensOverlayView>("lens-overlay-view")!;
  await view.updateComplete;
  return { view, button };
}

describe("overlay window dismissal", () => {
  it.each([
    "subscription-delayed",
    "snapshot-delayed",
    "subscription-failed",
    "snapshot-failed",
  ] as const)("closes an attached window when %s without inventing an operation", async (state) => {
    const pending = new Promise<never>(() => {});
    if (state === "subscription-delayed") port.subscribeToAppSnapshot.mockReturnValue(pending);
    if (state === "snapshot-delayed") port.getAppSnapshot.mockReturnValue(pending);
    if (state === "subscription-failed")
      port.subscribeToAppSnapshot.mockRejectedValue(new Error("Subscription unavailable"));
    if (state === "snapshot-failed")
      port.getAppSnapshot.mockRejectedValue(new Error("Snapshot unavailable"));
    const { view, button } = await attach();
    expect(view.model).toBeUndefined();
    await vi.waitFor(() =>
      expect(view.snapshotStatus.stage).toBe(state.endsWith("failed") ? "failed" : "loading"),
    );
    expect(button.disabled).toBe(false);
    expect(button.getAttribute("aria-label")).toBe("Close Lens");
    view.dispatchEvent(
      new CustomEvent("lens-overlay-intent", {
        detail: { type: "pause" },
        bubbles: true,
        composed: true,
      }),
    );
    expect(port.pauseLens).not.toHaveBeenCalled();
    button.click();
    await vi.waitFor(() => expect(port.closeCurrentWindow).toHaveBeenCalledTimes(1));
    expect(port.stopLens).not.toHaveBeenCalled();
  });

  it("closes a known idle snapshot without stopping an operation", async () => {
    const { view, button } = await attach();
    await vi.waitFor(() => expect(view.model).toBeDefined());
    button.click();
    await vi.waitFor(() => expect(port.closeCurrentWindow).toHaveBeenCalledTimes(1));
    expect(port.stopLens).not.toHaveBeenCalled();
  });

  it("waits for the known operation to stop before closing", async () => {
    const operationId = "0198e6de-d046-7bf2-b8b2-d84cfaba7e2d";
    port.getAppSnapshot.mockResolvedValue({
      ...snapshot,
      lens: { stage: "completed", operation_id: operationId, output_blocks: [] },
    });
    let completeStop!: () => void;
    port.stopLens.mockReturnValue(
      new Promise<void>((resolve) => {
        completeStop = resolve;
      }),
    );
    const { view, button } = await attach();
    await vi.waitFor(() => expect(view.model?.lens.operation_id).toBe(operationId));
    expect(button.getAttribute("aria-label")).toBe("Stop Lens and close");
    button.click();
    expect(port.stopLens).toHaveBeenCalledExactlyOnceWith(operationId);
    expect(port.closeCurrentWindow).not.toHaveBeenCalled();
    completeStop();
    await vi.waitFor(() => expect(port.closeCurrentWindow).toHaveBeenCalledTimes(1));
  });
});
