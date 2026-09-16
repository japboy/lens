import { LitElement, css, html, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import { repeat } from "lit/directives/repeat.js";
import type { DocumentBlock, SessionDocument } from "../application/session-document";
import { prepareHtmlPreview } from "../html-output";
import { imageDataUrl } from "../view-model";
import { externalMarkdownUrl } from "../markdown";
import { dispatchComponentEvent, OVERLAY_INTENT_EVENT } from "./events";
import "../streaming-markdown";

/** One renderer for canonical live and restored documents. It owns no agent commands. */
@customElement("lens-session-document")
export class LensSessionDocument extends LitElement {
  @property({ attribute: false }) document: SessionDocument | undefined;
  @property({ type: String }) identity = "session";
  static styles = css`
    :host {
      display: block;
      overflow-wrap: anywhere;
    }
    article {
      padding: 16px;
      border-bottom: 1px solid var(--border-color, #8884);
    }
    h3 {
      font: inherit;
      font-weight: 600;
      margin: 0 0 12px;
    }
    img {
      max-width: 100%;
      height: auto;
    }
    iframe {
      width: 100%;
      min-height: 360px;
      border: 0;
      background: white;
    }
    .status {
      font-size: 0.85em;
      opacity: 0.7;
    }
  `;
  protected render() {
    return this.document?.entries.length
      ? html`${repeat(
          this.document.entries,
          (e) => e.id,
          (entry) => html` <article data-entry-id=${entry.id}>
            <h3>
              ${entry.kind === "message" ? entry.role : entry.title}
              ${entry.kind === "tool" ? html`<span class="status"> ${entry.status}</span>` : nothing}
            </h3>
            ${entry.blocks.map((block, index) => this.renderBlock(block, `${this.identity}:${entry.id}:${index}`))}
            ${entry.kind === "tool" && entry.accepted_html ? this.renderBlock({ type: "html", text: entry.accepted_html }, `${this.identity}:${entry.id}:html`) : nothing}
          </article>`,
        )}`
      : html`<p>No session content is available.</p>`;
  }
  private renderBlock(block: DocumentBlock, identity: string) {
    switch (block.type) {
      case "markdown":
        return html`<lens-markdown
          .state=${{ operationId: identity, markdown: block.text, phase: "settled", scrollBehavior: "preserve" }}
          @click=${this.openLink}
        ></lens-markdown>`;
      case "image": {
        const source = imageDataUrl(block);
        return source
          ? html`<img src=${source} alt="Session image" />`
          : html`<p>Unsupported image type: ${block.mime_type}</p>`;
      }
      case "html": {
        try {
          const preview = prepareHtmlPreview(block.text);
          return html`<iframe
              title="Session HTML output"
              sandbox="allow-popups"
              referrerpolicy="no-referrer"
              .srcdoc=${preview.document}
            ></iframe>
            ${preview.notices.map((notice) => html`<p role="note">${notice}</p>`)}`;
        } catch (error) {
          return html`<p role="alert">Unable to display HTML: ${String(error)}</p>`;
        }
      }
      case "unsupported":
        return html`<p role="note">Unsupported content: ${block.content_type}</p>`;
    }
  }
  private openLink = (event: MouseEvent): void => {
    const link = event
      .composedPath()
      .find((node): node is HTMLAnchorElement => node instanceof HTMLAnchorElement);
    if (!link) return;
    event.preventDefault();
    const url = externalMarkdownUrl(link.getAttribute("href") ?? "");
    if (url) dispatchComponentEvent(this, OVERLAY_INTENT_EVENT, { type: "open-external-url", url });
  };
}
