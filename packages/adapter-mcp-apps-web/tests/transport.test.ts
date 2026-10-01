// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { isBoundedRpcMessage, OriginBoundAppTransport } from "../src/transport";
describe("App message admission", () => {
  it("rejects malformed methods and oversized serialized messages", () => {
    expect(isBoundedRpcMessage({ jsonrpc: "2.0", method: 7, id: 1 })).toBe(false);
    expect(
      isBoundedRpcMessage({
        jsonrpc: "2.0",
        method: "ui/message",
        params: { text: "a".repeat(4 * 1024 * 1024) },
      }),
    ).toBe(false);
    expect(isBoundedRpcMessage({ jsonrpc: "2.0", id: 1, result: {} })).toBe(true);
  });
  it("requires the exact proxy source/origin and removes its listener on close", async () => {
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const transport = new OriginBoundAppTransport(
      frame.contentWindow!,
      "http://127.0.0.1:43162",
      window,
    );
    transport.onmessage = vi.fn<NonNullable<OriginBoundAppTransport["onmessage"]>>();
    await transport.start();
    const data = { jsonrpc: "2.0", method: "ui/notifications/sandbox-proxy-ready", params: {} };
    const emit = (source: Window, origin: string) =>
      window.dispatchEvent(new MessageEvent("message", { source, origin, data }));
    emit(window, "http://127.0.0.1:43162");
    emit(frame.contentWindow!, "null");
    expect(transport.onmessage).not.toHaveBeenCalled();
    emit(frame.contentWindow!, "http://127.0.0.1:43162");
    expect(transport.onmessage).toHaveBeenCalledTimes(1);
    await transport.close();
    emit(frame.contentWindow!, "http://127.0.0.1:43162");
    expect(transport.onmessage).toHaveBeenCalledTimes(1);
    frame.remove();
  });
});
