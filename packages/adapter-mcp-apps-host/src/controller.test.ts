// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppBridge } from "@modelcontextprotocol/ext-apps/app-bridge";
import { APP_TEARDOWN_TIMEOUT_MS, McpAppController } from "./controller";
import type { McpAppDescriptor, McpAppLease, McpAppsPort, McpAppHostOptions } from "./types";

const descriptor = (id: string): McpAppDescriptor => ({
  id,
  operation_id: "operation",
  session_id: "session",
  server_id: "external",
  tool_name: "tool",
  resource_uri: "ui://test/view",
});
const lease = (id: string): McpAppLease => ({
  id: `lease-${id}`,
  artifact_id: id,
  proxy_url: "http://127.0.0.1:43162/proxy",
  proxy_origin: "http://127.0.0.1:43162",
  resource: { html: "<!doctype html><p>Original\u65e5\u672c\u8a9e</p>" },
  input: { value: 73 },
  result: { content: [], structuredContent: { value: 73 } },
  host_capabilities: { message: { text: {} }, updateModelContext: { text: {} }, serverTools: {} },
  live: true,
});
function setup() {
  const calls: string[] = [];
  const bridges: AppBridge[] = [];
  const port: McpAppsPort = {
    openMcpApp: vi.fn<McpAppsPort["openMcpApp"]>(async (id) => lease(id)),
    closeMcpApp: vi.fn<McpAppsPort["closeMcpApp"]>(async (id) => {
      calls.push(`revoke:${id}`);
    }),
    mcpAppRequest: vi.fn<McpAppsPort["mcpAppRequest"]>(async () => ({
      result: {},
      draft: { id: "draft-1", text: "Please explain 73" },
    })),
    submitMcpAppMessage: vi.fn<McpAppsPort["submitMcpAppMessage"]>(async () => {}),
    submitMcpAppLink: vi.fn<McpAppsPort["submitMcpAppLink"]>(async () => {}),
  };
  const factory = () => {
    const bridge = {
      connect: vi.fn<AppBridge["connect"]>(async () => {}),
      setRequestHandler: vi.fn<AppBridge["setRequestHandler"]>(),
      sendSandboxResourceReady: vi.fn<AppBridge["sendSandboxResourceReady"]>(async () => {}),
      sendToolInput: vi.fn<AppBridge["sendToolInput"]>(async () => {}),
      sendToolResult: vi.fn<AppBridge["sendToolResult"]>(async () => {}),
      teardownResource: vi.fn<AppBridge["teardownResource"]>(async () => {
        calls.push("teardown");
        return {};
      }),
      close: vi.fn<AppBridge["close"]>(async () => {
        calls.push("bridge-close");
      }),
    } as unknown as AppBridge;
    bridges.push(bridge);
    return bridge;
  };
  const container = document.createElement("div");
  document.body.append(container);
  const prepareDocument = vi.fn<NonNullable<McpAppHostOptions["prepareDocument"]>>(async () => {});
  const controller = new McpAppController(
    port,
    vi.fn<() => void>(),
    {
      hostInfo: { name: "Fixture Host", version: "1.0.0" },
      frameTitle: "Fixture App",
      getHostContext: () => ({}),
      prepareDocument,
    },
    factory,
  );
  const initialize = async () => {
    await bridges.at(-1)!.onsandboxready?.({});
    await bridges.at(-1)!.oninitialized?.({});
  };
  return { controller, port, bridges, container, calls, initialize, prepareDocument };
}
afterEach(() => {
  document.body.replaceChildren();
  vi.useRealTimers();
});

describe("selected App lifecycle", () => {
  it("awaits the injected preparer before mounting while retaining original result and input", async () => {
    const test = setup();
    const original = {
      ...lease("a"),
      input: { html: String.raw`<!doctype html><p>\(x\)</p><script>window.fixture=73</script>` },
      document_url: "http://127.0.0.1:43162/document/lease-a",
    };
    vi.mocked(test.port.openMcpApp).mockResolvedValueOnce(original);
    let resolve!: () => void;
    vi.mocked(test.prepareDocument).mockImplementationOnce(
      () =>
        new Promise<void>((done) => {
          resolve = done;
        }),
    );
    const opening = test.controller.show(descriptor("a"), test.container);
    await vi.waitFor(() => expect(test.prepareDocument).toHaveBeenCalledOnce());
    expect(test.container.querySelector("iframe")).toBeNull();
    const prepared = vi.mocked(test.prepareDocument).mock.calls[0]!;
    expect(prepared[0]).toBe(original);
    expect(prepared[1]).toEqual(descriptor("a"));
    resolve();
    await opening;
    await test.initialize();
    expect(test.bridges[0]!.sendToolInput).toHaveBeenCalledWith({ arguments: original.input });
    expect(test.bridges[0]!.sendToolResult).toHaveBeenCalledWith(original.result);
    await test.controller.close();
  });
  it.each(["late", "missing"])(
    "revokes a lease after Close with a %s injected preparation reply",
    async (reply) => {
      vi.useFakeTimers();
      const test = setup();
      vi.mocked(test.port.openMcpApp).mockResolvedValueOnce({
        ...lease("a"),
        input: { html: "<p>plain</p>" },
        document_url: "http://127.0.0.1:43162/document/lease-a",
      });
      let resolve!: () => void;
      vi.mocked(test.prepareDocument).mockImplementationOnce(
        () =>
          new Promise<void>((done) => {
            resolve = done;
          }),
      );
      const opening = test.controller.show(descriptor("a"), test.container);
      await vi.advanceTimersByTimeAsync(10);
      expect(test.prepareDocument).toHaveBeenCalledOnce();
      const close = test.controller.close();
      if (reply === "late") resolve();
      await vi.advanceTimersByTimeAsync(10_010);
      await opening;
      await close;
      expect(test.port.closeMcpApp).toHaveBeenCalledWith("lease-a");
      expect(test.bridges).toHaveLength(0);
      expect(test.container.querySelector("iframe")).toBeNull();
      await test.controller.show(descriptor("b"), test.container);
      await test.initialize();
      expect(test.controller.state.stage).toBe("ready");
      await test.controller.close();
    },
  );
  it("can show the next App after native close never resolves", async () => {
    vi.useFakeTimers();
    const test = setup();
    await test.controller.show(descriptor("a"), test.container);
    await test.initialize();
    vi.mocked(test.port.closeMcpApp).mockImplementation(() => new Promise(() => {}));
    const close = test.controller.close();
    await vi.advanceTimersByTimeAsync(1_010);
    await close;
    expect(test.container.querySelector("iframe")).toBeNull();
    await test.controller.show(descriptor("b"), test.container);
    await test.initialize();
    expect(test.controller.state.stage).toBe("ready");
    const finish = test.controller.close();
    await vi.advanceTimersByTimeAsync(1_010);
    await finish;
  });
  it("can open the next App after bridge cleanup rejects", async () => {
    const test = setup();
    await test.controller.show(descriptor("a"), test.container);
    await test.initialize();
    vi.mocked(test.bridges[0]!.close).mockRejectedValueOnce(new Error("Close failed"));
    await test.controller.close();
    expect(test.container.querySelector("iframe")).toBeNull();
    await test.controller.show(descriptor("b"), test.container);
    await test.initialize();
    expect(test.controller.state.stage).toBe("ready");
    await test.controller.close();
  });
  it("delivers original resource/input/result after handshake, revokes before teardown and detaches", async () => {
    const test = setup();
    await test.controller.show(descriptor("a"), test.container);
    await test.initialize();
    expect(test.bridges[0]!.sendSandboxResourceReady).toHaveBeenCalledWith({
      html: lease("a").resource.html,
      csp: undefined,
      permissions: {},
    });
    expect(test.bridges[0]!.sendToolInput).toHaveBeenCalledWith({ arguments: { value: 73 } });
    expect(test.bridges[0]!.sendToolResult).toHaveBeenCalledWith(lease("a").result);
    await test.controller.close();
    expect(test.calls.indexOf("revoke:lease-a")).toBeLessThan(test.calls.indexOf("teardown"));
    expect(test.container.querySelector("iframe")).toBeNull();
    expect(test.controller.state.stage).toBe("closed");
  });
  it("never dispatches ui/message until trusted submit and rejects a late result after close", async () => {
    const test = setup();
    await test.controller.show(descriptor("a"), test.container);
    await test.initialize();
    await test.bridges[0]!.onmessage?.(
      { role: "user", content: [{ type: "text", text: "Please explain 73" }] },
      {} as never,
    );
    expect(test.port.submitMcpAppMessage).not.toHaveBeenCalled();
    expect(test.controller.draft?.text).toBe("Please explain 73");
    await test.controller.submitDraft();
    expect(test.port.submitMcpAppMessage).toHaveBeenCalledWith("lease-a", "draft-1");
    let resolve!: (value: { result: unknown }) => void;
    vi.mocked(test.port.mcpAppRequest).mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const pending = test.bridges[0]!.oncalltool!({ name: "inspect", arguments: {} }, {} as never);
    const close = test.controller.close();
    resolve({ result: { content: [] } });
    await expect(pending).rejects.toThrow("App permission expired");
    await close;
  });
  it("closes an opening lease superseded before its resource can mount", async () => {
    const test = setup();
    let resolve!: (value: McpAppLease) => void;
    vi.mocked(test.port.openMcpApp).mockImplementationOnce(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const first = test.controller.show(descriptor("a"), test.container);
    await vi.waitFor(() => expect(test.port.openMcpApp).toHaveBeenCalledTimes(1));
    const second = test.controller.show(descriptor("b"), test.container);
    resolve(lease("a"));
    await first;
    await second;
    expect(test.port.closeMcpApp).toHaveBeenCalledWith("lease-a");
    expect(test.bridges).toHaveLength(1);
    expect(test.controller.state).toEqual({ stage: "initializing", artifactId: "b" });
    await test.controller.close();
  });
  it("bounds unacknowledged teardown and force-closes the transport/frame", async () => {
    vi.useFakeTimers();
    const test = setup();
    await test.controller.show(descriptor("a"), test.container);
    await test.initialize();
    vi.mocked(test.bridges[0]!.teardownResource).mockImplementation(() => new Promise(() => {}));
    // A foreign bridge may ignore its own timeout. The controller supplies a separate deadline.
    const close = test.controller.close();
    await vi.advanceTimersByTimeAsync(APP_TEARDOWN_TIMEOUT_MS + 10);
    await close;
    expect(test.container.querySelector("iframe")).toBeNull();
  });
  it("replaces twenty responsive Apps without retaining frames or active bridges", async () => {
    const test = setup();
    for (let index = 0; index < 20; index++) {
      await test.controller.show(descriptor(String(index)), test.container);
      await test.initialize();
      expect(test.container.querySelectorAll("iframe")).toHaveLength(1);
    }
    await test.controller.close();
    expect(test.container.querySelectorAll("iframe")).toHaveLength(0);
    expect(test.bridges.every((bridge) => vi.mocked(bridge.close).mock.calls.length === 1)).toBe(
      true,
    );
  });
});
