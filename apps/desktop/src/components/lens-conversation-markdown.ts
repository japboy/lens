import { GenerativeDom } from "@generative-dom/core";
import { markdownBase } from "@generative-dom/plugin-markdown-base";
import { markdownCode } from "@generative-dom/plugin-markdown-code";
import { markdownHeading } from "@generative-dom/plugin-markdown-heading";
import { markdownInline } from "@generative-dom/plugin-markdown-inline";
import { markdownLink } from "@generative-dom/plugin-markdown-link";
import { markdownList } from "@generative-dom/plugin-markdown-list";
import { markdownQuote } from "@generative-dom/plugin-markdown-quote";
import { markdownTable } from "@generative-dom/plugin-markdown-table";

/** Read-only transcript formatting. Fenced diagrams remain ordinary code. */
export class LensConversationMarkdown extends HTMLElement {
  private source = "";
  private rendered: string | undefined;
  private renderer: GenerativeDom | undefined;
  get markdown(): string {
    return this.source;
  }
  set markdown(value: string) {
    if (this.source === value) return;
    this.source = value;
    if (this.isConnected) this.renderContent();
  }
  connectedCallback(): void {
    this.renderContent();
  }
  disconnectedCallback(): void {
    if (!this.renderer) return;
    // GenerativeDom.destroy clears its container. Retain only the settled DOM,
    // while releasing its scheduler, token buffers and plugin state.
    const snapshot = document.createDocumentFragment();
    while (this.firstChild) snapshot.append(this.firstChild);
    this.renderer.destroy();
    this.renderer = undefined;
    this.append(snapshot);
  }
  private renderContent(): void {
    if (this.source === this.rendered) return;
    if (!this.renderer || this.source !== this.rendered) {
      this.renderer?.destroy();
      this.replaceChildren();
      this.rendered = "";
      this.renderer = new GenerativeDom({
        container: this,
        plugins: [
          markdownHeading(),
          markdownCode(),
          markdownQuote(),
          markdownList(),
          markdownTable(),
          markdownLink(),
          markdownInline(),
          markdownBase(),
        ],
        onError: (error) => {
          this.dispatchEvent(
            new CustomEvent("conversation-render-error", {
              detail: error.error.message,
              bubbles: true,
              composed: true,
            }),
          );
        },
      });
    }
    const suffix = this.source.slice(this.rendered?.length ?? 0);
    if (suffix) this.renderer.push(suffix);
    this.renderer.end();
    this.rendered = this.source;
  }
}
customElements.define("lens-conversation-markdown", LensConversationMarkdown);
