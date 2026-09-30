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
  const source = readFileSync(new URL("./rich-html-app.html", import.meta.url), "utf8");
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
  return { parent, frame, append, host, child };
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
