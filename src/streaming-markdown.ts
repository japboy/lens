import {
  GenerativeDom,
  createAutoScroller,
  type AutoScroller,
  type GenerativeDomError,
} from "@generative-dom/core";
import { markdownBase } from "@generative-dom/plugin-markdown-base";
import { markdownCode } from "@generative-dom/plugin-markdown-code";
import { cursor, type CursorPlugin } from "@generative-dom/plugin-cursor";
import { markdownHeading } from "@generative-dom/plugin-markdown-heading";
import { markdownInline } from "@generative-dom/plugin-markdown-inline";
import { markdownLink } from "@generative-dom/plugin-markdown-link";
import { markdownList } from "@generative-dom/plugin-markdown-list";
import { markdownQuote } from "@generative-dom/plugin-markdown-quote";
import { markdownTable } from "@generative-dom/plugin-markdown-table";
import { renderMarkdown } from "./markdown";

export type MarkdownRenderPhase = "streaming" | "settled";

export interface StreamingMarkdownState {
  operationId?: string;
  markdown: string;
  phase: MarkdownRenderPhase;
}

const EMPTY_STATE: StreamingMarkdownState = {
  markdown: "",
  phase: "settled",
};

/**
 * Owns an append-only Markdown DOM while ACP chunks are arriving.
 *
 * Lit owns this element, but deliberately does not own its descendants. This keeps
 * already committed Markdown nodes stable across LensState updates. A terminal state
 * is reparsed once through the canonical Marked + DOMPurify boundary.
 */
export class StreamingMarkdownElement extends HTMLElement {
  private pendingState: StreamingMarkdownState = EMPTY_STATE;
  private appliedState?: StreamingMarkdownState;
  private renderer?: GenerativeDom;
  private streamCursor?: CursorPlugin;
  private autoScroller?: AutoScroller;

  set state(next: StreamingMarkdownState) {
    this.pendingState = { ...next };
    if (this.isConnected) this.applyState(this.pendingState);
  }

  get state(): StreamingMarkdownState {
    return { ...this.pendingState };
  }

  connectedCallback(): void {
    this.autoScroller ??= createAutoScroller(this, { threshold: 36, smooth: false });
    this.applyState(this.pendingState);
  }

  disconnectedCallback(): void {
    this.disposeRenderer();
    this.autoScroller?.destroy();
    this.autoScroller = undefined;
    this.appliedState = undefined;
  }

  /** Forces scheduled streaming work to the DOM; useful for deterministic host tests. */
  flush(): void {
    this.renderer?.flush();
  }

  private applyState(next: StreamingMarkdownState): void {
    const previous = this.appliedState;
    const operationChanged = previous?.operationId !== next.operationId;
    if (operationChanged) {
      this.disposeRenderer();
      this.replaceChildren();
      this.appliedState = undefined;
    }

    if (next.phase === "settled") {
      if (this.appliedState?.phase !== "settled" || this.appliedState.markdown !== next.markdown) {
        this.renderSettled(next.markdown);
      }
      this.appliedState = { ...next };
      this.setAttribute("aria-busy", "false");
      this.removeAttribute("data-streaming");
      return;
    }

    const appliedText = this.appliedState?.phase === "streaming" ? this.appliedState.markdown : "";
    const appendOnly = next.markdown.startsWith(appliedText);
    if (!appendOnly || this.appliedState?.phase !== "streaming") {
      this.disposeRenderer();
      this.replaceChildren();
      this.createRenderer();
    } else if (!this.renderer) {
      this.createRenderer();
    }

    const baseLength =
      appendOnly && this.appliedState?.phase === "streaming" ? appliedText.length : 0;
    const delta = next.markdown.slice(baseLength);
    if (delta) this.renderer?.push(delta);
    this.streamCursor?.show();
    this.appliedState = { ...next };
    this.setAttribute("aria-busy", "true");
    this.setAttribute("data-streaming", "");
  }

  private createRenderer(): void {
    const streamCursor = cursor({
      character: "▍",
      className: "streaming-cursor",
      animated: !window.matchMedia("(prefers-reduced-motion: reduce)").matches,
    });
    this.streamCursor = streamCursor;
    this.renderer = new GenerativeDom({
      container: this,
      debounceMs: 16,
      maxLiveTokens: 256,
      plugins: [
        markdownHeading(),
        markdownCode(),
        markdownQuote(),
        markdownList(),
        markdownTable(),
        markdownLink(),
        markdownInline(),
        markdownBase(),
        streamCursor,
      ],
      onError: (error) => this.reportRenderError(error),
    });
    streamCursor.attach(this);
  }

  private renderSettled(markdown: string): void {
    if (this.renderer) this.renderer.end();
    this.disposeRenderer();
    this.innerHTML = renderMarkdown(markdown);
  }

  private disposeRenderer(): void {
    this.streamCursor?.hide();
    this.streamCursor = undefined;
    this.renderer?.destroy();
    this.renderer = undefined;
  }

  private reportRenderError(error: GenerativeDomError): void {
    this.dispatchEvent(
      new CustomEvent("markdown-render-error", {
        bubbles: true,
        composed: true,
        detail: `${error.phase}:${error.plugin}: ${error.error.message}`,
      }),
    );
  }
}

if (!customElements.get("personal-lens-markdown")) {
  customElements.define("personal-lens-markdown", StreamingMarkdownElement);
}
