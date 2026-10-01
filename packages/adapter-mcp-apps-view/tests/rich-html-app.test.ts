import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";

function shell(capabilities: Record<string, unknown>) {
  let receive!: (event: { source: unknown; origin: string; data: unknown }) => void;
  const parent = { postMessage: vi.fn<Window["postMessage"]>() };
  const frame = {
    contentWindow: { postMessage: vi.fn<Window["postMessage"]>() },
    setAttribute: vi.fn<Element["setAttribute"]>(),
    remove: vi.fn<Element["remove"]>(),
    src: "",
  };
  const status = { textContent: "", remove: vi.fn<Element["remove"]>() };
  const append = vi.fn<(frame: unknown) => void>();
  const source = readFileSync(new URL("../src/assets/rich-html-app.html", import.meta.url), "utf8");
  const script = source.match(/<script>([\s\S]*?)<\/script>/)![1]!;
  runInNewContext(script, {
    parent,
    URL,
    TextEncoder,
    window: {
      addEventListener: (_: string, handler: typeof receive) => {
        receive = handler;
      },
    },
    document: { getElementById: () => status, createElement: () => frame, body: { append } },
  });
  const host = (data: unknown) =>
    receive({ source: parent, origin: "http://127.0.0.1:43162", data });
  const child = (data: unknown) => receive({ source: frame.contentWindow, origin: "null", data });
  host({
    jsonrpc: "2.0",
    id: "lens-initialize",
    result: {
      hostCapabilities: capabilities,
      hostContext: { lensDocumentUrl: "http://127.0.0.1:43162/document/lease" },
    },
  });
  host({
    jsonrpc: "2.0",
    method: "ui/notifications/tool-result",
    params: {
      content: [],
      structuredContent: { html: "<!doctype html><script>unchanged</script>" },
    },
  });
  return { parent, frame, append, host, child, receive };
}

describe("bundled rich HTML App shell", () => {
  it("loads the original natural document URL and forwards only source-checked negotiated requests", () => {
    const test = shell({ message: { text: {} }, updateModelContext: { text: {} } });
    expect(test.frame.src).toBe("http://127.0.0.1:43162/document/lease");
    expect(test.frame.setAttribute).toHaveBeenCalledWith("sandbox", "allow-scripts allow-forms");
    expect(test.frame).not.toHaveProperty("srcdoc");
    const params = { role: "user", content: [{ type: "text", text: "Please explain 73" }] };
    test.child({ jsonrpc: "2.0", id: "child-1", method: "ui/message", params });
    expect(test.parent.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "lens-document-1", method: "ui/message", params },
      "*",
    );
    test.host({ jsonrpc: "2.0", id: "lens-document-1", result: {} });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "child-1", result: {} },
      "*",
    );
    const count = test.parent.postMessage.mock.calls.length;
    test.child({ jsonrpc: "2.0", id: 4, method: "tools/call", params: { name: "unavailable" } });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(count);
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({
        id: 4,
        error: { code: -32601, message: "App operation unavailable" },
      }),
      "*",
    );
  });
  it("forwards standard open-link for a read-only App only when its capability is negotiated", () => {
    const test = shell({ openLinks: {} });
    const params = { url: "https://example.com/73" };
    test.child({ jsonrpc: "2.0", id: "anchor-1", method: "ui/open-link", params });
    expect(test.parent.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "lens-document-1", method: "ui/open-link", params },
      "*",
    );
    test.host({ jsonrpc: "2.0", id: "lens-document-1", result: { isError: false } });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "anchor-1", result: { isError: false } },
      "*",
    );
    test.child({ jsonrpc: "2.0", id: "context", method: "ui/update-model-context", params: {} });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({
        id: "context",
        error: { code: -32601, message: "App operation unavailable" },
      }),
      "*",
    );
  });
  it("rejects foreign frames, nonopaque children, unmatched and replaced-document responses", () => {
    const test = shell({ openLinks: {} });
    const request = {
      jsonrpc: "2.0",
      id: "same-child-id",
      method: "ui/open-link",
      params: { url: "https://example.com" },
    };
    const before = test.parent.postMessage.mock.calls.length;
    test.receive({ source: {}, origin: "null", data: request });
    test.receive({
      source: test.frame.contentWindow,
      origin: "https://example.com",
      data: request,
    });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(before);
    test.child(request);
    test.child(request);
    expect(
      test.parent.postMessage.mock.calls
        .slice(-2)
        .map(([message]) => (message as { id: string }).id),
    ).toEqual(["lens-document-1", "lens-document-2"]);
    const replies = test.frame.contentWindow.postMessage.mock.calls.length;
    test.receive({
      source: {},
      origin: "http://127.0.0.1:43162",
      data: { jsonrpc: "2.0", id: "lens-document-1", result: { isError: false } },
    });
    test.host({ jsonrpc: "2.0", id: "unmatched", result: {} });
    expect(test.frame.contentWindow.postMessage).toHaveBeenCalledTimes(replies);
    test.host({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: {} });
    test.host({ jsonrpc: "2.0", id: "lens-document-1", result: { isError: false } });
    test.host({ jsonrpc: "2.0", id: "lens-document-2", result: { isError: false } });
    expect(test.frame.contentWindow.postMessage).toHaveBeenCalledTimes(replies);
    test.child(request);
    test.host({ jsonrpc: "2.0", id: "lens-document-3", result: { isError: false } });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "same-child-id", result: { isError: false } },
      "*",
    );
    test.host({ jsonrpc: "2.0", id: "teardown", method: "ui/resource-teardown", params: {} });
    const closedReplies = test.frame.contentWindow.postMessage.mock.calls.length;
    test.host({ jsonrpc: "2.0", id: "lens-document-3", result: { isError: false } });
    expect(test.frame.contentWindow.postMessage).toHaveBeenCalledTimes(closedReplies);
  });
  it("keeps retained historical content read-only and revokes pending messages on teardown", () => {
    const test = shell({});
    const count = test.parent.postMessage.mock.calls.length;
    test.child({ jsonrpc: "2.0", id: 2, method: "ui/message", params: {} });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(count);
    test.host({ jsonrpc: "2.0", id: "teardown", method: "ui/resource-teardown", params: {} });
    expect(test.frame.remove).toHaveBeenCalledOnce();
    expect(test.parent.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "teardown", result: {} },
      "*",
    );
    const after = test.parent.postMessage.mock.calls.length;
    test.child({ jsonrpc: "2.0", id: 3, method: "ui/message", params: {} });
    test.host({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: {} });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(after);
    expect(test.append).toHaveBeenCalledOnce();
  });
});
