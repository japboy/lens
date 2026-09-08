import { LitElement, css, html, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { renderStaticHtml, safeHtmlLink } from "../html-output";

@customElement("lens-html-output")
export class LensHtmlOutput extends LitElement {
  @property({ attribute: false }) resourceId = "";
  @property({ attribute: false }) content: string | undefined;
  @property({ attribute: false }) status: "loading" | "ready" | "failed" = "loading";
  @property({ attribute: false }) failureMessage: string | undefined;
  @state() private renderError = "";

  static styles = css`
    :host {
      display: block;
      height: 100%;
      min-height: 0;
      contain: layout paint style;
    }
    :host([hidden]) {
      display: none;
    }
    .viewport {
      box-sizing: border-box;
      height: 100%;
      overflow: auto;
      background: #fff;
      color: #20242a;
      font:
        14px/1.5 system-ui,
        sans-serif;
      overflow-wrap: anywhere;
    }
    .content {
      box-sizing: border-box;
      contain: layout paint style;
      min-height: 100%;
      padding: var(--html-output-padding, 52px 54px 22px);
    }
    @media (max-width: 480px) {
      .content {
        padding: var(--html-output-padding, 52px 38px 22px);
      }
    }
    .message {
      padding: 56px 24px;
    }
  `;

  protected updated(changed: PropertyValues<this>): void {
    if (!changed.has("resourceId") && !changed.has("content") && !changed.has("status")) return;
    const target = this.renderRoot.querySelector(".content")!;
    target.replaceChildren();
    const viewport = this.renderRoot.querySelector<HTMLElement>(".viewport")!;
    viewport.scrollTop = 0;
    viewport.scrollLeft = 0;
    this.renderError = "";
    if (this.status !== "ready") return;
    try {
      if (this.content === undefined) throw new Error("HTML content is missing.");
      target.append(renderStaticHtml(this.content));
      this.dispatchEvent(
        new CustomEvent("html-render-ready", {
          detail: { resourceId: this.resourceId },
          bubbles: true,
          composed: true,
        }),
      );
    } catch (error) {
      this.renderError = error instanceof Error ? error.message : "HTML could not be displayed.";
      this.dispatchEvent(
        new CustomEvent("html-render-error", {
          detail: { resourceId: this.resourceId, message: this.renderError },
          bubbles: true,
          composed: true,
        }),
      );
    }
  }

  private openLink(event: MouseEvent): void {
    const anchor = event.composedPath().find((node) => node instanceof HTMLAnchorElement) as
      | HTMLAnchorElement
      | undefined;
    if (!anchor) return;
    event.preventDefault();
    const url = safeHtmlLink(anchor.getAttribute("href") ?? "");
    if (url)
      this.dispatchEvent(
        new CustomEvent("html-open-link", { detail: { url }, bubbles: true, composed: true }),
      );
  }

  protected render() {
    const message =
      this.renderError ||
      (this.status === "failed"
        ? this.failureMessage || "HTML could not be loaded."
        : this.status === "loading"
          ? "Loading HTML…"
          : "");
    return html`<div class="message" role="status" ?hidden=${!message}>${message}</div>
      <div
        class="viewport"
        role="region"
        aria-label="HTML content"
        tabindex="0"
        ?hidden=${!!message}
        aria-busy=${this.status === "loading" ? "true" : "false"}
        @click=${this.openLink}
        @auxclick=${this.openLink}
      >
        <div class="content"></div>
      </div>`;
  }
}
