import { AppBridge, type McpUiHostCapabilities } from "@modelcontextprotocol/ext-apps/app-bridge";
import { OriginBoundAppTransport } from "./transport";
import { version } from "../../package.json";
import type {
  McpAppDescriptor,
  McpAppLease,
  McpAppMessageDraft,
  McpAppsPort,
  McpAppState,
} from "./types";

export const APP_INITIALIZATION_TIMEOUT_MS = 10_000;
export const APP_TEARDOWN_TIMEOUT_MS = 1_000;
const BACKEND_CLOSE_TIMEOUT_MS = 1_000;

type BridgeFactory = (
  capabilities: McpUiHostCapabilities,
  lease: McpAppLease,
  owner: Window,
) => AppBridge;
interface MountedApp {
  lease: McpAppLease;
  frame: HTMLIFrameElement;
  bridge: AppBridge;
  transport: OriginBoundAppTransport;
  initialized: boolean;
  sandboxReady: boolean;
  revoked: boolean;
  initializationTimer: ReturnType<typeof setTimeout>;
}

function hostCapabilities(source: McpUiHostCapabilities): McpUiHostCapabilities {
  // Only advertise implementations available through the native source-bound port.
  return {
    ...(source.serverTools ? { serverTools: source.serverTools } : {}),
    ...(source.serverResources ? { serverResources: source.serverResources } : {}),
    ...(source.message?.text ? { message: { text: {} } } : {}),
    ...(source.updateModelContext?.text || source.updateModelContext?.structuredContent
      ? {
          updateModelContext: {
            ...(source.updateModelContext.text ? { text: {} } : {}),
            ...(source.updateModelContext.structuredContent ? { structuredContent: {} } : {}),
          },
        }
      : {}),
    sandbox: { csp: source.sandbox?.csp ?? {}, permissions: {} },
  };
}

function defaultBridge(
  capabilities: McpUiHostCapabilities,
  lease: McpAppLease,
  owner: Window,
): AppBridge {
  return new AppBridge(null, { name: "Lens", version }, capabilities, {
    hostContext: {
      platform: "desktop",
      theme: owner.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light",
      displayMode: "inline",
      availableDisplayModes: ["inline"],
      ...(lease.document_url ? { lensDocumentUrl: lease.document_url } : {}),
    },
  });
}

async function bounded<T>(promise: Promise<T>, milliseconds: number): Promise<T | undefined> {
  let timer: number | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<undefined>((resolve) => {
        timer = window.setTimeout(resolve, milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

/** One selected App. Replacement first revokes authority, then disposes its document. */
export class McpAppController {
  state: McpAppState = { stage: "idle" };
  draft: McpAppMessageDraft | undefined;
  submitting = false;
  submissionError = "";
  private epoch = 0;
  private mounted: MountedApp | undefined;
  private operation: Promise<void> = Promise.resolve();
  private cleanup: Promise<void> | undefined;
  constructor(
    private readonly port: McpAppsPort,
    private readonly changed: () => void,
    private readonly createBridge: BridgeFactory = defaultBridge,
  ) {}

  private setState(state: McpAppState): void {
    this.state = state;
    this.changed();
  }

  show(descriptor: McpAppDescriptor, container: HTMLElement): Promise<void> {
    const epoch = ++this.epoch;
    this.revokeCurrent();
    this.operation = this.operation
      .catch(() => {})
      .then(async () => {
        await this.disposeMounted();
        if (epoch !== this.epoch) return;
        this.setState({ stage: "opening", artifactId: descriptor.id });
        let lease: McpAppLease | undefined;
        try {
          const owner = container.ownerDocument.defaultView;
          if (!owner) throw new Error("App document is unavailable");
          lease = await this.port.openMcpApp(descriptor.id, owner.location.origin);
          if (epoch !== this.epoch || !container.isConnected) {
            await bounded(this.port.closeMcpApp(lease.id), BACKEND_CLOSE_TIMEOUT_MS);
            return;
          }
          const proxy = new URL(lease.proxy_url);
          if (
            lease.artifact_id !== descriptor.id ||
            proxy.origin !== lease.proxy_origin ||
            proxy.origin === owner.location.origin ||
            proxy.protocol !== "http:" ||
            proxy.hostname !== "127.0.0.1"
          )
            throw new Error("Invalid App sandbox origin");
          const frame = container.ownerDocument.createElement("iframe");
          frame.title = descriptor.title ?? "Interactive Interpretation";
          frame.className = "output-html-frame";
          frame.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms");
          frame.setAttribute("referrerpolicy", "no-referrer");
          frame.style.cssText = "border:0;width:100%;height:100%;display:block";
          container.append(frame);
          if (!frame.contentWindow) throw new Error("App frame is unavailable");
          const capabilities = hostCapabilities(lease.host_capabilities);
          const bridge = this.createBridge(capabilities, lease, owner);
          const transport = new OriginBoundAppTransport(frame.contentWindow, proxy.origin, owner);
          const mounted: MountedApp = {
            lease,
            frame,
            bridge,
            transport,
            initialized: false,
            sandboxReady: false,
            revoked: false,
            initializationTimer: setTimeout(() => {
              if (this.mounted !== mounted || mounted.initialized) return;
              this.setState({
                stage: "failed",
                artifactId: descriptor.id,
                message: "App initialization timed out. Reopen the App to try again.",
              });
              this.revokeCurrent();
              void this.disposeMounted();
            }, APP_INITIALIZATION_TIMEOUT_MS),
          };
          this.mounted = mounted;
          this.setState({ stage: "initializing", artifactId: descriptor.id });
          if (capabilities.serverTools) {
            bridge.oncalltool = (params) =>
              this.request(mounted, "tools/call", params) as ReturnType<
                NonNullable<AppBridge["oncalltool"]>
              >;
            bridge.setRequestHandler(
              "tools/list",
              (request) =>
                this.request(mounted, "tools/list", request.params) as Promise<{ tools: never[] }>,
            );
          }
          if (capabilities.serverResources) {
            bridge.onlistresources = (params) =>
              this.request(mounted, "resources/list", params) as ReturnType<
                NonNullable<AppBridge["onlistresources"]>
              >;
            bridge.onreadresource = (params) =>
              this.request(mounted, "resources/read", params) as ReturnType<
                NonNullable<AppBridge["onreadresource"]>
              >;
          }
          if (capabilities.message)
            bridge.onmessage = (params) =>
              this.request(mounted, "ui/message", params) as ReturnType<
                NonNullable<AppBridge["onmessage"]>
              >;
          if (capabilities.updateModelContext)
            bridge.onupdatemodelcontext = (params) =>
              this.request(mounted, "ui/update-model-context", params) as ReturnType<
                NonNullable<AppBridge["onupdatemodelcontext"]>
              >;
          bridge.onrequestteardown = () => {
            if (this.mounted === mounted) void this.close();
          };
          bridge.onsandboxready = async () => {
            this.assertCurrent(mounted, false);
            if (mounted.sandboxReady) throw new Error("Duplicate sandbox initialization");
            mounted.sandboxReady = true;
            await bridge.sendSandboxResourceReady({
              html: lease!.resource.html,
              csp: lease!.resource.meta?.ui?.csp,
              permissions: {},
            });
          };
          bridge.oninitialized = async () => {
            this.assertCurrent(mounted, false);
            if (!mounted.sandboxReady || mounted.initialized)
              throw new Error("Invalid App initialization order");
            mounted.initialized = true;
            clearTimeout(mounted.initializationTimer);
            await bridge.sendToolInput({ arguments: lease!.input });
            this.assertCurrent(mounted);
            await bridge.sendToolResult(lease!.result);
            this.assertCurrent(mounted);
            this.setState({ stage: "ready", artifactId: descriptor.id, live: lease!.live });
          };
          bridge.onerror = () => {
            if (this.mounted !== mounted || mounted.revoked) return;
            // Keep payloads and protocol details out of product errors and console logs.
            this.setState({
              stage: "failed",
              artifactId: descriptor.id,
              message: "The App connection failed. Reopen the App to try again.",
            });
            this.revokeCurrent();
            void this.disposeMounted();
          };
          await bridge.connect(transport);
          this.assertCurrent(mounted, false);
          frame.src = lease.proxy_url;
        } catch (error) {
          if (this.mounted) await this.disposeMounted();
          else if (lease)
            await bounded(
              this.port.closeMcpApp(lease.id).catch(() => {}),
              BACKEND_CLOSE_TIMEOUT_MS,
            );
          if (epoch === this.epoch)
            this.setState({ stage: "failed", artifactId: descriptor.id, message: String(error) });
        }
      });
    return this.operation;
  }

  private assertCurrent(mounted: MountedApp, requireInitialized = true): void {
    if (this.mounted !== mounted || mounted.revoked || (requireInitialized && !mounted.initialized))
      throw new Error("App permission expired");
  }
  private async request(mounted: MountedApp, method: string, params: unknown): Promise<unknown> {
    this.assertCurrent(mounted);
    const response = await this.port.mcpAppRequest(mounted.lease.id, { method, params });
    this.assertCurrent(mounted);
    if (response.draft) {
      this.draft = response.draft;
      this.submissionError = "";
      this.changed();
    }
    return response.result;
  }
  discardDraft(): void {
    this.draft = undefined;
    this.submissionError = "";
    this.changed();
  }
  async submitDraft(): Promise<void> {
    const mounted = this.mounted;
    const draft = this.draft;
    if (!mounted || !draft || this.submitting) return;
    this.assertCurrent(mounted);
    this.submitting = true;
    this.submissionError = "";
    this.changed();
    try {
      await this.port.submitMcpAppMessage(mounted.lease.id, draft.id);
      this.assertCurrent(mounted);
      if (this.draft === draft) this.draft = undefined;
    } catch (error) {
      if (this.mounted === mounted && !mounted.revoked) this.submissionError = String(error);
    } finally {
      this.submitting = false;
      this.changed();
    }
  }
  private revokeCurrent(): void {
    const mounted = this.mounted;
    if (!mounted || mounted.revoked) return;
    mounted.revoked = true;
    this.draft = undefined;
    this.submissionError = "";
    clearTimeout(mounted.initializationTimer);
    // Start native revocation immediately; do not wait for a cooperative App.
    void this.port.closeMcpApp(mounted.lease.id).catch(() => {});
  }
  private async disposeMounted(): Promise<void> {
    const mounted = this.mounted;
    if (!mounted) {
      await this.cleanup;
      return;
    }
    this.mounted = undefined;
    mounted.revoked = true;
    clearTimeout(mounted.initializationTimer);
    const cleanup = (async () => {
      try {
        await bounded(
          this.port.closeMcpApp(mounted.lease.id).catch(() => {}),
          BACKEND_CLOSE_TIMEOUT_MS,
        );
      } finally {
        try {
          await bounded(
            mounted.bridge.teardownResource({}, { timeout: APP_TEARDOWN_TIMEOUT_MS }),
            APP_TEARDOWN_TIMEOUT_MS,
          );
        } catch {
          /* Cooperative cleanup is bounded; document destruction always follows. */
        } finally {
          try {
            await bounded(mounted.bridge.close(), APP_TEARDOWN_TIMEOUT_MS);
          } catch {
            // Cleanup failure cannot poison the serialized replacement queue.
            // The owned transport and document are still released below.
            this.submissionError = "The App connection could not close cleanly. You can reopen it.";
          } finally {
            await mounted.transport.close();
            mounted.frame.remove();
          }
        }
      }
    })();
    this.cleanup = cleanup;
    try {
      await cleanup;
    } finally {
      if (this.cleanup === cleanup) this.cleanup = undefined;
    }
  }
  close(): Promise<void> {
    ++this.epoch;
    const artifactId = "artifactId" in this.state ? this.state.artifactId : undefined;
    this.revokeCurrent();
    if (artifactId) this.setState({ stage: "closing", artifactId });
    this.operation = this.operation
      .catch(() => {})
      .then(async () => {
        await this.disposeMounted();
        if (artifactId) this.setState({ stage: "closed", artifactId });
        else this.setState({ stage: "idle" });
      });
    return this.operation;
  }
}
