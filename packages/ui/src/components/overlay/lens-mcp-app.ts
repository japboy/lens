import { createUiMcpAppController } from "../../resources/document-host";
import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { McpAppController, type McpAppDescriptor, type McpAppState } from "adapter-mcp-apps-host";
import {
  type UiMcpAppsPort as McpAppsPort,
  type HtmlPresentationSource,
} from "../../contracts/document-host";

/** Presentation only: native authority arrives through the page-owned narrow port. */
@customElement("lens-mcp-app")
export class LensMcpApp extends LitElement {
  @property({ attribute: false }) descriptor:
    | McpAppDescriptor
    | { kind: "html"; id: string; title?: string; source: HtmlPresentationSource }
    | undefined;
  @property({ attribute: false }) port: McpAppsPort | undefined;
  @property({ type: Boolean }) active = false;
  @state() private stopped = false;
  private controller: McpAppController | undefined;
  private openingIdentity: string | undefined;
  private container: HTMLElement | undefined;

  protected createRenderRoot(): HTMLElement {
    return this;
  }
  connectedCallback(): void {
    super.connectedCallback();
    // A cached tab can reconnect without changed inputs. Re-evaluate document ownership.
    this.requestUpdate();
  }
  protected updated(changed: PropertyValues): void {
    if (
      changed.has("descriptor") &&
      (changed.get("descriptor") as typeof this.descriptor)?.id !== this.descriptor?.id
    )
      this.stopped = false;
    if (changed.has("port")) {
      void this.controller?.close();
      this.controller = this.port
        ? createUiMcpAppController(this.port, () => {
            this.requestUpdate();
            this.dispatchEvent(
              new CustomEvent("lens-mcp-app-state", { bubbles: true, composed: true }),
            );
          })
        : undefined;
      this.openingIdentity = undefined;
    }
    if (!this.container) {
      this.container = this.ownerDocument.createElement("div");
      this.container.className = "mcp-app-frame-container";
      this.container.style.cssText = "position:relative;min-height:0;height:100%;flex:1";
      this.querySelector(".mcp-app-document")?.append(this.container);
    }
    const identity =
      this.isConnected && this.active && !this.stopped ? this.descriptor?.id : undefined;
    if (identity === this.openingIdentity) return;
    this.openingIdentity = identity;
    if (identity && this.descriptor && this.controller && this.container) {
      const descriptor = this.descriptor;
      if ("kind" in descriptor)
        void this.controller.showDocument(descriptor, this.container, (origin) =>
          this.port!.openHtmlPresentation(descriptor.source, origin),
        );
      else void this.controller.show(descriptor, this.container);
    } else void this.controller?.close();
  }
  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.openingIdentity = undefined;
    void this.controller?.close();
  }
  async dispose(): Promise<void> {
    this.stopped = true;
    this.openingIdentity = undefined;
    await this.controller?.close();
  }
  get stage(): McpAppState["stage"] {
    return this.controller?.state.stage ?? "idle";
  }
  protected render() {
    const state = this.controller?.state;
    return html`<div
        class="mcp-app-document"
        style="flex:1;min-height:0;display:flex;flex-direction:column;overflow:hidden"
      ></div>
      <div class="mcp-app-controls">
        ${state?.stage === "opening" || state?.stage === "initializing" ? html`<p role="status">Loading interactive Interpretation…</p>` : nothing}
        ${state?.stage === "failed" ? html`<p class="output-media-error" role="alert">${state.message}</p>` : nothing}
        ${this.controller?.connectionError ? html`<p class="output-media-error" role="alert">${this.controller.connectionError}</p>` : nothing}
        ${
          this.active && (this.stopped || state?.stage === "failed" || state?.stage === "closed")
            ? html`<button
                type="button"
                data-lens-button-role="normal"
                ?disabled=${state?.stage === "closing"}
                @click=${() => {
                  this.stopped = false;
                  this.openingIdentity = undefined;
                  this.requestUpdate();
                }}
              >
                Reopen App
              </button>`
            : nothing
        }
      </div>`;
  }
}
