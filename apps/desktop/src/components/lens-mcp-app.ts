import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { McpAppController } from "../mcp-apps/controller";
import type { McpAppDescriptor, McpAppsPort } from "../mcp-apps/types";

/** Presentation only: native authority arrives through the page-owned narrow port. */
@customElement("lens-mcp-app")
export class LensMcpApp extends LitElement {
  @property({ attribute: false }) descriptor: McpAppDescriptor | undefined;
  @property({ attribute: false }) port: McpAppsPort | undefined;
  @property({ type: Boolean }) active = false;
  @property({ type: Boolean }) replay = false;
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
      (changed.get("descriptor") as McpAppDescriptor | undefined)?.id !== this.descriptor?.id
    )
      this.stopped = false;
    if (changed.has("port")) {
      void this.controller?.close();
      this.controller = this.port
        ? new McpAppController(this.port, () => {
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
    if (identity && this.descriptor && this.controller && this.container)
      void this.controller.show(this.descriptor, this.container);
    else void this.controller?.close();
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
  get ready(): boolean {
    return this.controller?.state.stage === "ready";
  }
  protected render() {
    const state = this.controller?.state;
    const draft = this.controller?.draft;
    const readOnly = this.replay || (state?.stage === "ready" && !state.live);
    return html`<div
        class="mcp-app-document"
        style="flex:1;min-height:0;display:flex;flex-direction:column;overflow:hidden"
      ></div>
      <div class="mcp-app-controls">
        ${readOnly ? html`<p role="status">This saved App is read-only. Its agent connection is closed.</p>` : nothing}
        ${state?.stage === "opening" || state?.stage === "initializing" ? html`<p role="status">Loading interactive Interpretation…</p>` : nothing}
        ${state?.stage === "failed" ? html`<p class="output-media-error" role="alert">${state.message}</p>` : nothing}
        ${this.controller?.submissionError ? html`<p class="output-media-error" role="alert">${this.controller.submissionError}</p>` : nothing}
        ${
          draft
            ? html`<section aria-label="Message to agent">
                <p>${draft.text}</p>
                <button
                  type="button"
                  data-lens-button-role="primary"
                  ?disabled=${this.controller?.submitting}
                  @click=${() => void this.controller?.submitDraft()}
                >
                  Send to Agent</button
                ><button
                  type="button"
                  data-lens-button-role="normal"
                  ?disabled=${this.controller?.submitting}
                  @click=${() => this.controller?.discardDraft()}
                >
                  Discard
                </button>
              </section>`
            : nothing
        }
        ${
          this.active
            ? html`<button
                type="button"
                data-lens-button-role="normal"
                ?disabled=${state?.stage === "closing"}
                @click=${() => {
                  if (this.stopped || state?.stage === "failed") {
                    this.stopped = false;
                    this.openingIdentity = undefined;
                    this.requestUpdate();
                  } else void this.dispose();
                }}
              >
                ${this.stopped || state?.stage === "failed" ? "Reopen App" : "Close App"}
              </button>`
            : nothing
        }
      </div>`;
  }
}
