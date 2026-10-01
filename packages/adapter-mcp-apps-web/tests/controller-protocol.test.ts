// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { McpAppController } from "../src/controller";
import type { McpAppsPort } from "../src/types";

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it.each([true, false])(
  "uses the real pinned AppBridge handshake with source-bound live=%s authority and exact native shapes",
  async (live) => {
    vi.stubGlobal(
      "matchMedia",
      vi.fn<Window["matchMedia"]>(() => ({ matches: live }) as MediaQueryList),
    );
    // jsdom replaces contentWindow on src assignment, unlike browser WindowProxy.
    // Network navigation is outside this protocol test; keep the transport peer stable.
    vi.spyOn(HTMLIFrameElement.prototype, "src", "set").mockImplementation(() => {});
    const port: McpAppsPort = {
      openMcpApp: vi.fn<McpAppsPort["openMcpApp"]>(async () => ({
        id: "lease",
        artifact_id: "artifact",
        generation: "gen",
        proxy_url: "http://127.0.0.1:43162/proxy",
        proxy_origin: "http://127.0.0.1:43162",
        resource: { html: "<!doctype html><p>App</p>" },
        input: { selection: 73, nested: { text: "\u65e5\u672c\u8a9e" } },
        result: { content: [], structuredContent: { selection: 73 } },
        host_capabilities: {
          openLinks: {},
          sandbox: {
            csp: {
              resourceDomains: ["https://cdn.example"],
              connectDomains: ["https://api.example"],
            },
            permissions: {},
          },
          ...(live
            ? {
                serverTools: {},
                message: { text: {} },
                updateModelContext: { text: {}, structuredContent: {} },
              }
            : {}),
        },
        live,
      })),
      closeMcpApp: vi.fn<McpAppsPort["closeMcpApp"]>(async () => {}),
      mcpAppRequest: vi.fn<McpAppsPort["mcpAppRequest"]>(async () => ({ result: { tools: [] } })),
      submitMcpAppMessage: vi.fn<McpAppsPort["submitMcpAppMessage"]>(async () => {}),
      submitMcpAppLink: vi.fn<McpAppsPort["submitMcpAppLink"]>(async () => {}),
    };
    const container = document.createElement("div");
    document.body.append(container);
    const controller = new McpAppController(port, vi.fn<() => void>(), {
      hostInfo: { name: "Injected Host", version: "1.2.3" },
      frameTitle: "Injected App",
      getHostContext: (_, owner) => ({
        theme: owner.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
      }),
    });
    await controller.show(
      {
        id: "artifact",
        operation_id: "op",
        session_id: "session",
        server_id: "server",
        tool_name: "render",
        resource_uri: "ui://app",
      },
      container,
    );
    const target = container.querySelector("iframe")!.contentWindow!;
    const received: Record<string, unknown>[] = [];
    const emit = (data: unknown) =>
      window.dispatchEvent(
        new MessageEvent("message", { source: target, origin: "http://127.0.0.1:43162", data }),
      );
    vi.spyOn(target, "postMessage").mockImplementation((message) => {
      const value = message as Record<string, unknown>;
      received.push(value);
      if (value.method === "ui/resource-teardown")
        queueMicrotask(() => emit({ jsonrpc: "2.0", id: value.id, result: {} }));
    });
    emit({ jsonrpc: "2.0", method: "ui/notifications/sandbox-proxy-ready", params: {} });
    await vi.waitFor(() =>
      expect(
        received.some((message) => message.method === "ui/notifications/sandbox-resource-ready"),
      ).toBe(true),
    );
    emit({
      jsonrpc: "2.0",
      id: 1,
      method: "ui/initialize",
      params: {
        appInfo: { name: "Official protocol fixture", version: "1.0.0" },
        appCapabilities: {},
        protocolVersion: "2026-01-26",
      },
    });
    await vi.waitFor(() =>
      expect(received.some((message) => message.id === 1 && message.result)).toBe(true),
    );
    const initializeResult = received.find((message) => message.id === 1)?.result as {
      hostInfo: { name: string; version: string };
      hostContext: { theme: string };
      hostCapabilities: {
        updateModelContext?: {
          text?: Record<string, never>;
          structuredContent?: Record<string, never>;
        };
      };
    };
    expect(initializeResult.hostInfo).toEqual({ name: "Injected Host", version: "1.2.3" });
    expect(initializeResult.hostContext.theme).toBe(live ? "dark" : "light");
    expect(window.matchMedia).toHaveBeenCalledWith("(prefers-color-scheme: dark)");
    expect(initializeResult.hostCapabilities.updateModelContext).toEqual(
      live ? { text: {}, structuredContent: {} } : undefined,
    );
    expect(initializeResult.hostCapabilities).toMatchObject({
      sandbox: {
        csp: { resourceDomains: ["https://cdn.example"], connectDomains: ["https://api.example"] },
        permissions: {},
      },
    });
    emit({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} });
    await vi.waitFor(() => expect(controller.state.stage).toBe("ready"));
    expect(
      received.find((message) => message.method === "ui/notifications/tool-input")?.params,
    ).toEqual({ arguments: { selection: 73, nested: { text: "\u65e5\u672c\u8a9e" } } });
    expect(
      received.find((message) => message.method === "ui/notifications/tool-result")?.params,
    ).toEqual({ content: [], structuredContent: { selection: 73 } });
    emit({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} });
    await vi.waitFor(() => expect(received.find((message) => message.id === 2)).toBeDefined());
    const toolsResponse = received.find((message) => message.id === 2)!;
    expect(toolsResponse.result).toEqual(live ? { tools: [] } : undefined);
    expect("error" in toolsResponse).toBe(!live);
    const context = {
      content: [{ type: "text", text: "Selection 73" }],
      structuredContent: { selected_value: 73 },
    };
    vi.mocked(port.mcpAppRequest).mockResolvedValue({ result: {} });
    emit({ jsonrpc: "2.0", id: 3, method: "ui/update-model-context", params: context });
    await vi.waitFor(() => expect(received.find((message) => message.id === 3)).toBeDefined());
    const contextResponse = received.find((message) => message.id === 3)!;
    expect(contextResponse.result).toEqual(live ? {} : undefined);
    expect("error" in contextResponse).toBe(!live);
    expect(vi.mocked(port.mcpAppRequest).mock.calls).toEqual(
      live
        ? [
            ["lease", { method: "tools/list", params: {} }],
            ["lease", { method: "ui/update-model-context", params: context }],
          ]
        : [],
    );
    expect(initializeResult.hostCapabilities).toMatchObject({ openLinks: {} });
    vi.mocked(port.mcpAppRequest).mockResolvedValueOnce({
      result: { isError: false },
      link: { id: "link-1", url: "https://example.com/73" },
    });
    emit({
      jsonrpc: "2.0",
      id: 4,
      method: "ui/open-link",
      params: { url: "https://example.com/73" },
    });
    await vi.waitFor(() => expect(received.find((message) => message.id === 4)).toBeDefined());
    expect(received.find((message) => message.id === 4)?.result).toEqual({ isError: false });
    expect(controller.link).toEqual({ id: "link-1", url: "https://example.com/73" });
    expect(port.submitMcpAppLink).not.toHaveBeenCalled();
    await controller.submitLink();
    expect(port.submitMcpAppLink).toHaveBeenCalledWith("lease", "link-1");
    expect(controller.link).toBeUndefined();
    expect(port.submitMcpAppMessage).not.toHaveBeenCalled();
    await controller.close();
    expect(container.querySelector("iframe")).toBeNull();
  },
);
