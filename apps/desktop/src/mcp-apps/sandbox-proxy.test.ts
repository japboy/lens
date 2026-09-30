import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";

function proxy(hostOrigin = "http://localhost:1420") {
  let receive!: (event: { source: unknown; origin: string; data: unknown }) => Promise<void>;
  const frames: {
    contentWindow: { postMessage: ReturnType<typeof vi.fn> };
    remove: ReturnType<typeof vi.fn>;
  }[] = [];
  const parent = { postMessage: vi.fn<Window["postMessage"]>() };
  const top = Object.defineProperty({}, "document", {
    get() {
      throw Object.assign(new Error("Denied"), { name: "SecurityError" });
    },
  });
  const fetch = vi.fn<(url: string, init?: RequestInit) => Promise<{ ok: boolean }>>(async () => ({
    ok: true,
  }));
  const context = {
    window: {
      addEventListener: (_: string, handler: typeof receive) => {
        receive = handler;
      },
    },
    parent,
    top,
    self: {},
    location: { href: "http://127.0.0.1:43162/proxy" },
    URL,
    TextEncoder,
    AbortController,
    fetch,
    document: {
      getElementById: () => ({
        textContent: JSON.stringify({ hostOrigin, viewUrl: "/view/lease" }),
      }),
      createElement: () => ({
        contentWindow: { postMessage: vi.fn<Window["postMessage"]>() },
        setAttribute: vi.fn<Element["setAttribute"]>(),
        style: {},
        remove: vi.fn<Element["remove"]>(),
      }),
      body: { append: (frame: (typeof frames)[number]) => frames.push(frame), textContent: "" },
    },
  };
  runInNewContext(readFileSync(new URL("./sandbox-proxy.js", import.meta.url), "utf8"), context);
  const host = (data: unknown) => receive({ source: parent, origin: hostOrigin, data });
  return { fetch, parent, frames, host, receive };
}
describe("trusted sandbox proxy asset", () => {
  it("uploads exact UTF-8 resource as text/html and only forwards the opaque View", async () => {
    const test = proxy();
    const html = "<!doctype html><title>\u65e5\u672c\u8a9e</title><script>let x=73</script>";
    await test.host({
      jsonrpc: "2.0",
      method: "ui/notifications/sandbox-resource-ready",
      params: { html },
    });
    expect(test.fetch).toHaveBeenCalledWith(
      "/view/lease",
      expect.objectContaining({
        method: "POST",
        headers: { "content-type": "text/html" },
        body: html,
      }),
    );
    expect(test.frames).toHaveLength(1);
    const request = {
      jsonrpc: "2.0",
      id: 3,
      method: "ui/message",
      params: { role: "user", content: [{ type: "text", text: "73" }] },
    };
    await test.receive({ source: test.frames[0]!.contentWindow, origin: "null", data: request });
    expect(test.parent.postMessage).toHaveBeenCalledWith(request, "http://localhost:1420");
    const calls = test.parent.postMessage.mock.calls.length;
    await test.receive({
      source: test.frames[0]!.contentWindow,
      origin: "null",
      data: { jsonrpc: "2.0", method: "ui/notifications/sandbox-proxy-ready", params: {} },
    });
    expect(test.parent.postMessage.mock.calls).toHaveLength(calls);
  });
  it("does not create an old View after teardown races its pending resource upload", async () => {
    const test = proxy();
    let resolve!: (value: { ok: boolean }) => void;
    test.fetch.mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const load = test.host({
      jsonrpc: "2.0",
      method: "ui/notifications/sandbox-resource-ready",
      params: { html: "original" },
    });
    await test.host({ jsonrpc: "2.0", id: 10, method: "ui/resource-teardown", params: {} });
    resolve({ ok: true });
    await load;
    expect(test.frames).toHaveLength(0);
    expect(test.parent.postMessage).toHaveBeenCalledWith(
      { jsonrpc: "2.0", id: 10, result: {} },
      "http://localhost:1420",
    );
  });
  it("uses wildcard destination only for an opaque host and still checks exact parent source", async () => {
    const test = proxy("null");
    expect(test.parent.postMessage.mock.calls[0]?.[1]).toBe("*");
    await test.receive({
      source: {},
      origin: "null",
      data: {
        jsonrpc: "2.0",
        method: "ui/notifications/sandbox-resource-ready",
        params: { html: "stolen" },
      },
    });
    expect(test.fetch).not.toHaveBeenCalled();
  });
});
