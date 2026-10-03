import { toUiMcpAppsPort } from "../../src/mcp-apps/composition";
// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { AppBridge, type McpAppDescriptor } from "adapter-mcp-apps-host";
import { ResponseHistoryController } from "ui/resources/response-history-controller";
import type { WebviewPort } from "../../src/application/webview-port";
import type { LensOverlayView } from "ui/components/views/lens-overlay-view";
import type { LensMcpApp } from "ui/components/overlay/lens-mcp-app";
import type { DesktopMcpAppsPort } from "../../src/mcp-apps/composition";
import type { LensResponseManifest, LensState } from "ui/contracts/lens";

beforeAll(async () => {
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  HTMLElement.prototype.scrollTo ??= () => undefined;
  await import("ui/components/overlay/lens-agent-output");
  await import("ui/components/views/lens-overlay-view");
});
afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

const app: McpAppDescriptor = {
  id: "artifact",
  operation_id: "op",
  session_id: "session",
  server_id: "external",
  tool_name: "display",
  resource_uri: "ui://external/app",
};
function response(sequence: number): LensResponseManifest {
  return {
    representation_id: `r${sequence}`,
    sequence,
    run_id: `run-${sequence}`,
    prompt_execution_revision: 1,
    context_id: "context",
    context_revision: sequence,
    projection: { revision: sequence, digest: "fixture" },
    retained_bytes: 3,
    block_count: sequence === 1 ? 1 : 0,
    blocks: sequence === 1 ? [{ type: "markdown", block_index: 0, byte_length: 3 }] : [],
    ...(sequence === 2 ? { mcp_apps: [app] } : {}),
  };
}
function lens(count: number): LensState {
  return {
    operation_id: "op",
    stage: "completed",
    prompt_execution_revision: 1,
    output_blocks: [],
    response_history: {
      responses: Array.from({ length: count }, (_, index) => response(index + 1)),
      retained_bytes: 3,
      capacity_reached: false,
    },
    live: {
      lifecycle: "watching",
      health: "healthy",
      freshness: "current",
      agent_refresh_interval_seconds: 180,
    },
  };
}
async function mount() {
  // jsdom does not create browsing contexts for frames inside an Overlay shadow root.
  // The SDK handshake is driven below; native frame isolation has separate acceptance.
  vi.spyOn(HTMLIFrameElement.prototype, "contentWindow", "get").mockReturnValue(window);
  const connect = vi.spyOn(AppBridge.prototype, "connect").mockResolvedValue();
  vi.spyOn(AppBridge.prototype, "teardownResource").mockResolvedValue({});
  vi.spyOn(AppBridge.prototype, "close").mockResolvedValue();
  vi.spyOn(AppBridge.prototype, "sendSandboxResourceReady").mockResolvedValue();
  vi.spyOn(AppBridge.prototype, "sendToolInput").mockResolvedValue();
  vi.spyOn(AppBridge.prototype, "sendToolResult").mockResolvedValue();
  const port: DesktopMcpAppsPort = {
    openMcpApp: vi.fn<DesktopMcpAppsPort["openMcpApp"]>(async () => ({
      id: "lease",
      artifact_id: app.id,
      document_mode: null,
      proxy_url: "http://127.0.0.1:43162/proxy",
      proxy_origin: "http://127.0.0.1:43162",
      resource: { html: "<p>External App</p>" },
      input: {},
      result: { content: [] },
      host_capabilities: {},
      live: true,
    })),
    closeMcpApp: vi.fn<DesktopMcpAppsPort["closeMcpApp"]>(async () => {}),
    mcpAppRequest: vi.fn<DesktopMcpAppsPort["mcpAppRequest"]>(async () => ({ result: {} })),
    openHtmlPresentation: vi.fn<DesktopMcpAppsPort["openHtmlPresentation"]>(),
    prepareMcpAppDocument: vi.fn<DesktopMcpAppsPort["prepareMcpAppDocument"]>(async () => {}),
  };
  const view = document.createElement("lens-overlay-view") as LensOverlayView;
  const history = new ResponseHistoryController(view, {
    getResponseBlock: vi.fn<WebviewPort["getResponseBlock"]>(async () => ({
      type: "markdown",
      text: "abc",
    })),
  });
  const publish = (count: number) => {
    const snapshot = lens(count);
    history.synchronize(snapshot);
    view.model = {
      platform: "macos",
      pending: false,
      cancelPending: false,
      message: "",
      lens: snapshot,
    };
    view.responseHistory = history.presentation;
  };
  view.active = true;
  view.appPort = toUiMcpAppsPort(port);
  view.loadResponseBlock = history.loadBlock;
  publish(1);
  document.body.append(view);
  await view.updateComplete;
  return { view, port, connect, publish };
}

describe("response-scoped MCP App navigation", () => {
  it("opens and acknowledges an App-only follow-up using the same media projection as the carousel", async () => {
    const { view, port, connect, publish } = await mount();
    const root = view.shadowRoot!;
    expect(root.querySelector(".lens-view-latest")).toBeNull();
    publish(2);
    await view.updateComplete;
    await vi.waitFor(() => expect(connect).toHaveBeenCalledOnce());
    const bridge = connect.mock.contexts[0] as AppBridge;
    await bridge.onsandboxready?.({});
    await bridge.oninitialized?.({});
    await vi.waitFor(() =>
      expect(root.querySelector<LensMcpApp>("lens-mcp-app")?.stage).toBe("ready"),
    );
    expect(root.querySelector(".overlay-shell")?.getAttribute("data-media-cue")).toBe("true");
    expect(view.responseHistory?.media.map((item) => item.id)).toEqual(["app:artifact"]);
    expect(root.querySelector(".overlay-new-response-count")?.textContent).toBe("1 new response");
    root.querySelector<HTMLButtonElement>(".lens-view-latest")!.click();
    await vi.waitFor(() => expect(root.querySelector(".overlay-new-response-count")).toBeNull());
    expect(root.querySelector(".lens-update-error")).toBeNull();
    expect(
      root.querySelector('lens-output-media .output-media-slide[aria-hidden="false"] lens-mcp-app'),
    ).not.toBeNull();
    expect(root.activeElement?.classList.contains("output-media-details-toggle")).toBe(true);
    expect(port.openMcpApp).toHaveBeenCalledExactlyOnceWith(app.id, window.location.origin);
    expect(port.prepareMcpAppDocument).not.toHaveBeenCalled();
  });

  it("finishes failed App navigation without acknowledging an unread response", async () => {
    const { view, port, publish } = await mount();
    vi.mocked(port.openMcpApp).mockRejectedValue(new Error("App resource unavailable"));
    publish(2);
    await view.updateComplete;
    const root = view.shadowRoot!;
    await vi.waitFor(() =>
      expect(root.querySelector<LensMcpApp>("lens-mcp-app")?.stage).toBe("failed"),
    );
    root.querySelector<HTMLButtonElement>(".lens-view-latest")!.click();
    await vi.waitFor(() =>
      expect(root.querySelector(".lens-update-error")?.textContent).toContain(
        "could not be opened",
      ),
    );
    expect(root.querySelector(".overlay-new-response-count")?.textContent).toBe("1 new response");
    expect(root.querySelector<HTMLButtonElement>(".lens-view-latest")?.disabled).toBe(false);
  });
});
