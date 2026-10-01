// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import type { McpAppDescriptor, McpAppLease } from "adapter-mcp-apps-host";
import { version } from "../../package.json";
import {
  createDesktopMcpAppController,
  type DesktopMcpAppsPort,
} from "../../src/mcp-apps/composition";

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it.each(["external", "builtin", "static", "interactive-replay"] as const)(
  "composes the real Host SDK with desktop metadata and %s presentation",
  async (kind) => {
    const prepares = kind !== "external";
    const mode = kind === "static" ? "static" : prepares ? "interactive" : null;
    const isDocument = kind === "static" || kind === "interactive-replay";
    vi.stubGlobal(
      "matchMedia",
      vi.fn<Window["matchMedia"]>(() => ({ matches: true }) as MediaQueryList),
    );
    vi.spyOn(HTMLIFrameElement.prototype, "src", "set").mockImplementation(() => {});
    const raw = String.raw`<!doctype html><p>\(x\)</p><script>window.author = 73</script>`;
    const descriptor: McpAppDescriptor = {
      id: "artifact",
      operation_id: "operation",
      session_id: "session",
      server_id: kind === "builtin" ? "lens_rich_content" : "external",
      tool_name: "render",
      resource_uri: "ui://fixture",
    };
    const lease: McpAppLease = {
      id: "lease",
      artifact_id: isDocument ? null : "artifact",
      document_mode: mode,
      proxy_url: "http://127.0.0.1:43162/proxy",
      proxy_origin: "http://127.0.0.1:43162",
      resource: { html: "<!doctype html><p>App shell</p>" },
      input: { html: raw },
      result: { content: [], structuredContent: { html: raw } },
      host_capabilities: {},
      live: false,
      ...(prepares ? { document_url: "http://127.0.0.1:43162/document/lease" } : {}),
    };
    const port: DesktopMcpAppsPort = {
      openMcpApp: vi.fn<DesktopMcpAppsPort["openMcpApp"]>(async () => lease),
      closeMcpApp: vi.fn<DesktopMcpAppsPort["closeMcpApp"]>(async () => {}),
      openHtmlPresentation: vi.fn<DesktopMcpAppsPort["openHtmlPresentation"]>(async () => lease),
      prepareMcpAppDocument: vi.fn<DesktopMcpAppsPort["prepareMcpAppDocument"]>(async () => {}),
      mcpAppRequest: vi.fn<DesktopMcpAppsPort["mcpAppRequest"]>(async () => ({ result: {} })),
    };
    const container = document.createElement("div");
    document.body.append(container);
    const controller = createDesktopMcpAppController(port, vi.fn<() => void>());
    const source = {
      kind: "history" as const,
      generation: "saved-generation",
      entry_id: "html-entry",
      revision: 7,
      block_index: 2,
    };
    if (isDocument)
      await controller.showDocument({ id: "saved-html" }, container, (origin) =>
        port.openHtmlPresentation(source, origin),
      );
    else await controller.show(descriptor, container);
    const preparations = vi.mocked(port.prepareMcpAppDocument).mock.calls;
    expect(preparations).toHaveLength(prepares ? 1 : 0);
    expect(preparations[0]?.[0]).toBe(prepares ? "lease" : undefined);
    const derivative = preparations[0]?.[1] ?? "";
    expect(derivative.includes('class="katex"')).toBe(prepares);
    expect(derivative.includes("data:font/woff2;base64,")).toBe(prepares);
    expect(derivative.includes("<script>window.author = 73</script>")).toBe(mode === "interactive");
    expect(lease.input.html).toBe(raw);
    expect(lease.result.structuredContent).toEqual({ html: raw });
    const target = container.querySelector("iframe")!.contentWindow!;
    const received: Record<string, unknown>[] = [];
    const emit = (data: unknown) =>
      window.dispatchEvent(
        new MessageEvent("message", {
          source: target,
          origin: lease.proxy_origin,
          data,
        }),
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
        appInfo: { name: "Fixture", version: "1.0.0" },
        appCapabilities: {},
        protocolVersion: "2026-01-26",
      },
    });
    await vi.waitFor(() =>
      expect(received.find((message) => message.id === 1)?.result).toBeDefined(),
    );
    expect(received.find((message) => message.id === 1)?.result).toMatchObject({
      hostInfo: { name: "Lens", version },
      hostContext: {
        theme: "dark",
        platform: "desktop",
        displayMode: "inline",
        availableDisplayModes: ["inline"],
        ...(prepares ? { lensDocumentUrl: lease.document_url, lensDocumentMode: mode } : {}),
      },
    });
    emit({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} });
    await vi.waitFor(() => expect(controller.state.stage).toBe("ready"));
    expect(
      received.find((message) => message.method === "ui/notifications/tool-input")?.params,
    ).toEqual({ arguments: lease.input });
    expect(
      received.find((message) => message.method === "ui/notifications/tool-result")?.params,
    ).toEqual(lease.result);
    await controller.close();
  },
);
