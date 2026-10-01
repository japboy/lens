// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { AppBridge } from "adapter-mcp-apps-host";
import type { DesktopMcpAppsPort as McpAppsPort } from "../../src/mcp-apps/composition";
import type { PresentedOutputMedia } from "../../src/output-media";
import type { LensOutputMedia } from "../../src/components/lens-output-media";
beforeAll(async () => {
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  await import("../../src/components/lens-output-media");
});
afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});
async function mount(media: readonly PresentedOutputMedia[]): Promise<LensOutputMedia> {
  const element = document.createElement("lens-output-media") as LensOutputMedia;
  element.media = media;
  document.body.append(element);
  await element.updateComplete;
  return element;
}
describe("Interpretation media App composition", () => {
  it("registers and mounts an App through the production media owner without directly importing its leaf", async () => {
    expect(customElements.get("lens-mcp-app")).toBeTypeOf("function");
    vi.spyOn(AppBridge.prototype, "teardownResource").mockResolvedValue({});
    const descriptor = {
      id: "artifact",
      operation_id: "operation",
      session_id: "session",
      server_id: "server",
      tool_name: "tool",
      resource_uri: "ui://test/app",
    };
    const port: McpAppsPort = {
      openMcpApp: vi.fn<McpAppsPort["openMcpApp"]>(async () => ({
        id: "lease",
        artifact_id: descriptor.id,
        proxy_url: "http://127.0.0.1:43162/proxy",
        proxy_origin: "http://127.0.0.1:43162",
        resource: { html: "<p>Original App</p>" },
        input: { value: 73 },
        result: { content: [] },
        host_capabilities: {},
        live: true,
      })),
      closeMcpApp: vi.fn<McpAppsPort["closeMcpApp"]>(async () => {}),
      mcpAppRequest: vi.fn<McpAppsPort["mcpAppRequest"]>(async () => ({ result: {} })),
      submitMcpAppMessage: vi.fn<McpAppsPort["submitMcpAppMessage"]>(async () => {}),
      submitMcpAppLink: vi.fn<McpAppsPort["submitMcpAppLink"]>(async () => {}),
      prepareMcpAppDocument: vi.fn<McpAppsPort["prepareMcpAppDocument"]>(async () => {}),
    };
    const element = await mount([
      { kind: "app", id: "app:artifact", mimeType: "text/html;profile=mcp-app", descriptor },
    ]);
    element.appPort = port;
    await element.updateComplete;
    await vi.waitFor(() =>
      expect(port.openMcpApp).toHaveBeenCalledExactlyOnceWith("artifact", window.location.origin),
    );
    const app = element.querySelector<HTMLElement & { dispose(): Promise<void> }>("lens-mcp-app")!;
    await vi.waitFor(() => expect(app.querySelector("iframe")).not.toBeNull());
    expect(app.textContent).not.toContain("Close App");
    await app.dispose();
    expect(app.querySelector("iframe")).toBeNull();
  });
});
