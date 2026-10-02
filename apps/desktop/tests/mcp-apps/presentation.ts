// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { AppBridge } from "adapter-mcp-apps-host";
import { html, render } from "lit";
import { cache } from "lit/directives/cache.js";
import { LensMcpApp } from "../../src/components/lens-mcp-app";
import type { McpAppLease } from "adapter-mcp-apps-host";
import type { DesktopMcpAppsPort as McpAppsPort } from "../../src/mcp-apps/composition";

const lease: McpAppLease = {
  id: "lease",
  artifact_id: "artifact",
  document_mode: null,
  proxy_url: "http://127.0.0.1:43162/proxy",
  proxy_origin: "http://127.0.0.1:43162",
  resource: { html: "<p>Saved content</p>" },
  input: {},
  result: { content: [] },
  host_capabilities: {},
  live: false,
};
const flush = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};
function mount(port: McpAppsPort) {
  vi.spyOn(AppBridge.prototype, "connect").mockResolvedValue();
  vi.spyOn(AppBridge.prototype, "teardownResource").mockResolvedValue({});
  vi.spyOn(AppBridge.prototype, "close").mockResolvedValue();
  const element = new LensMcpApp();
  element.descriptor = {
    id: "artifact",
    operation_id: "operation",
    session_id: "session",
    server_id: "server",
    tool_name: "tool",
    resource_uri: "ui://test/app",
  };
  element.port = port;
  element.active = true;
  document.body.append(element);
  return element;
}
function port(): McpAppsPort {
  return {
    openMcpApp: vi.fn<McpAppsPort["openMcpApp"]>(async () => lease),
    closeMcpApp: vi.fn<McpAppsPort["closeMcpApp"]>(async () => {}),
    mcpAppRequest: vi.fn<McpAppsPort["mcpAppRequest"]>(async () => ({ result: {} })),
    openHtmlPresentation: vi.fn<McpAppsPort["openHtmlPresentation"]>(),
    prepareMcpAppDocument: vi.fn<McpAppsPort["prepareMcpAppDocument"]>(async () => {}),
  };
}
afterEach(async () => {
  document.body.replaceChildren();
  await flush();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("App presentation lifecycle", () => {
  it("routes read-only App links directly to the Host without an agent message or second prompt", async () => {
    const native = port();
    vi.mocked(native.openMcpApp).mockResolvedValueOnce({
      ...lease,
      host_capabilities: { openLinks: {} },
    });
    vi.mocked(native.mcpAppRequest).mockResolvedValueOnce({
      result: { isError: false },
    });
    const element = mount(native);

    vi.spyOn(AppBridge.prototype, "sendSandboxResourceReady").mockResolvedValue();
    vi.spyOn(AppBridge.prototype, "sendToolInput").mockResolvedValue();
    vi.spyOn(AppBridge.prototype, "sendToolResult").mockResolvedValue();
    await element.updateComplete;
    await flush();
    const bridge = vi.mocked(AppBridge.prototype.connect).mock.contexts.at(-1)! as AppBridge;
    await bridge.onsandboxready?.({});
    await bridge.oninitialized?.({});
    await bridge.onopenlink?.({ url: "https://example.com/path?q=73" }, {} as never);
    await element.updateComplete;
    expect(native.mcpAppRequest).toHaveBeenCalledWith("lease", {
      method: "ui/open-link",
      params: { url: "https://example.com/path?q=73" },
    });
    expect(element.querySelector('section[aria-label="Open external link"]')).toBeNull();
    await element.dispose();
  });
  it("passes an App message directly to native admission without a Send confirmation", async () => {
    const native = port();
    vi.mocked(native.openMcpApp).mockResolvedValueOnce({
      ...lease,
      live: true,
      host_capabilities: { message: { text: {} } },
    });
    const element = mount(native);
    vi.spyOn(AppBridge.prototype, "sendSandboxResourceReady").mockResolvedValue();
    vi.spyOn(AppBridge.prototype, "sendToolInput").mockResolvedValue();
    vi.spyOn(AppBridge.prototype, "sendToolResult").mockResolvedValue();
    await element.updateComplete;
    await flush();
    const bridge = vi.mocked(AppBridge.prototype.connect).mock.contexts.at(-1)! as AppBridge;
    await bridge.onsandboxready?.({});
    await bridge.oninitialized?.({});
    const params = {
      role: "user" as const,
      content: [{ type: "text" as const, text: "Explain selection 73" }],
    };
    await expect(bridge.onmessage!(params, {} as never)).resolves.toEqual({});
    expect(native.mcpAppRequest).toHaveBeenCalledExactlyOnceWith("lease", {
      method: "ui/message",
      params,
    });
    await element.updateComplete;
    expect(element.querySelector('section[aria-label="Message to agent"]')).toBeNull();
    expect(element.textContent).not.toContain("Send to Agent");
    await element.dispose();
  });
  it("releases the App while a tab is cached and remounts after reconnect without changed inputs", async () => {
    const native = port();
    const element = mount(native);
    const tabs = document.createElement("div");
    document.body.append(tabs);
    const interpretation = () =>
      render(cache(html`<section id="interpretation">${element}</section>`), tabs);
    const conversation = () =>
      render(cache(html`<section id="conversation">Conversation</section>`), tabs);
    interpretation();
    await element.updateComplete;
    await flush();
    expect(native.openMcpApp).toHaveBeenCalledTimes(1);
    expect(element.querySelector("iframe")).not.toBeNull();
    for (let transition = 0; transition < 3; transition++) {
      conversation();
      await flush();
      await element.updateComplete;
      expect(element.isConnected).toBe(false);
      await vi.waitFor(() => expect(element.querySelector("iframe")).toBeNull());
      expect(native.closeMcpApp).toHaveBeenCalledWith("lease");
      expect(AppBridge.prototype.teardownResource).toHaveBeenCalled();
      expect(AppBridge.prototype.close).toHaveBeenCalled();
      // Native cleanup schedules Lit updates even in the disconnected cached tree.
      element.requestUpdate();
      await element.updateComplete;
      await flush();
      expect(native.openMcpApp).toHaveBeenCalledTimes(transition + 1);
      interpretation();
      await element.updateComplete;
      await flush();
      expect(element.isConnected).toBe(true);
      await vi.waitFor(() => expect(element.querySelector("iframe")).not.toBeNull());
      expect(native.openMcpApp).toHaveBeenCalledTimes(transition + 2);
    }
    await element.dispose();
  });
  it("mounts an immutable saved App without opening agent or tool authority", async () => {
    const native = port();
    const element = mount(native);
    await element.updateComplete;
    await flush();
    expect(native.openMcpApp).toHaveBeenCalledWith("artifact", window.location.origin);
    expect(element.querySelector("iframe")).not.toBeNull();
    expect(element.textContent).not.toContain("This App’s agent connection is closed.");
    expect(native.mcpAppRequest).not.toHaveBeenCalled();
    await element.dispose();
  });
  it("enables the actual Reopen control after native close never resolves", async () => {
    vi.useFakeTimers();
    const native = port();
    vi.mocked(native.closeMcpApp).mockImplementation(() => new Promise(() => {}));
    const element = mount(native);
    await element.updateComplete;
    await flush();
    expect(element.textContent).not.toContain("Close App");
    const disposal = element.dispose();
    await flush();
    await vi.advanceTimersByTimeAsync(1_010);
    await disposal;
    await element.updateComplete;
    expect(element.querySelector("iframe")).toBeNull();
    const reopen = element.querySelector("button") as HTMLButtonElement;
    expect(reopen.textContent).toContain("Reopen App");
    expect(reopen.disabled).toBe(false);
    // Unrelated native snapshots must not clear the explicit local Closed state.
    element.descriptor = { ...element.descriptor! };
    await element.updateComplete;
    expect(native.openMcpApp).toHaveBeenCalledTimes(1);
    reopen.click();
    await element.updateComplete;
    await flush();
    expect(native.openMcpApp).toHaveBeenCalledTimes(2);
    expect(element.querySelector("iframe")).not.toBeNull();
    const close = element.dispose();
    await vi.advanceTimersByTimeAsync(1_010);
    await close;
  });
});
