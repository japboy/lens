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
import { dispatchComponentEvent } from "./components/events";
import { renderMarkdown } from "./markdown";
import { prefersReducedMotion } from "./styles/component-styles";
import { renderMermaidCodeBlocks, type MermaidTheme } from "./mermaid";

export type MarkdownRenderPhase = "streaming" | "settled";

export interface StreamingMarkdownState {
  operationId?: string;
  markdown: string;
  phase: MarkdownRenderPhase;
  scrollBehavior?: "follow" | "preserve";
}

const EMPTY_STATE: StreamingMarkdownState = {
  markdown: "",
  phase: "settled",
};
const AUTO_SCROLL_CONTAINER_SELECTOR = "[data-auto-scroll-container]";

/**
 * Owns an append-only Markdown DOM while ACP chunks are arriving.
 *
 * Lit owns this element, but deliberately does not own its descendants. This keeps
 * already committed Markdown nodes stable across LensState updates. A terminal state
 * is reparsed once through the canonical Marked + DOMPurify boundary.
 */
export class StreamingMarkdownElement extends HTMLElement {
  private static nextInstanceId = 1;

  private readonly instanceId = StreamingMarkdownElement.nextInstanceId++;
  private pendingState: StreamingMarkdownState = EMPTY_STATE;
  private appliedState?: StreamingMarkdownState;
  private renderer?: GenerativeDom;
  private streamCursor?: CursorPlugin;
  private autoScroller?: AutoScroller;
  private colorScheme?: MediaQueryList;
  private mermaidRevision = 0;

  set state(next: StreamingMarkdownState) {
    this.pendingState = { ...next };
    if (this.isConnected) this.applyState(this.pendingState);
  }

  get state(): StreamingMarkdownState {
    return { ...this.pendingState };
  }

  connectedCallback(): void {
    this.colorScheme ??= window.matchMedia("(prefers-color-scheme: dark)");
    this.colorScheme.addEventListener?.("change", this.handleColorSchemeChange);
    this.applyState(this.pendingState);
  }

  disconnectedCallback(): void {
    this.mermaidRevision += 1;
    this.disposeRenderer();
    this.autoScroller?.destroy();
    this.autoScroller = undefined;
    this.colorScheme?.removeEventListener?.("change", this.handleColorSchemeChange);
    this.colorScheme = undefined;
    this.appliedState = undefined;
  }

  /** Forces scheduled streaming work to the DOM; useful for deterministic host tests. */
  flush(): void {
    this.renderer?.flush();
  }

  private applyState(next: StreamingMarkdownState): void {
    if (next.scrollBehavior === "preserve") {
      this.autoScroller?.destroy();
      this.autoScroller = undefined;
    } else if (!this.autoScroller) {
      const container = this.closest<HTMLElement>(AUTO_SCROLL_CONTAINER_SELECTOR) ?? this;
      this.autoScroller = createAutoScroller(container, { threshold: 36, smooth: false });
    }
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
      this.removeAttribute("data-streaming");
      return;
    }

    this.mermaidRevision += 1;
    this.removeAttribute("data-mermaid-state");

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
      animated: !prefersReducedMotion(),
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
    const revision = ++this.mermaidRevision;
    if (this.renderer) this.renderer.end();
    this.disposeRenderer();
    this.innerHTML = renderMarkdown(markdown);

    const hasMermaid = this.querySelector("pre > code.language-mermaid") !== null;
    if (!hasMermaid) {
      this.setAttribute("aria-busy", "false");
      this.removeAttribute("data-mermaid-state");
      return;
    }

    this.setAttribute("aria-busy", "true");
    this.dataset.mermaidState = "rendering";
    const theme: MermaidTheme = this.colorScheme?.matches ? "dark" : "default";
    void renderMermaidCodeBlocks(this, {
      idPrefix: `lens-mermaid-${this.instanceId}-${revision}`,
      isCurrent: () => this.isConnected && revision === this.mermaidRevision,
      theme,
    })
      .then((outcome) => {
        if (outcome.status === "stale" || revision !== this.mermaidRevision) return;
        this.setAttribute("aria-busy", "false");
        this.dataset.mermaidState = outcome.errors.length > 0 ? "error" : "ready";
        if (outcome.errors.length > 0) this.reportMermaidErrors(outcome.errors);
      })
      .catch((reason) => {
        if (revision !== this.mermaidRevision) return;
        this.setAttribute("aria-busy", "false");
        this.dataset.mermaidState = "error";
        this.reportMermaidErrors([reason instanceof Error ? reason : new Error(String(reason))]);
      });
  }

  private disposeRenderer(): void {
    this.streamCursor?.hide();
    this.streamCursor = undefined;
    this.renderer?.destroy();
    this.renderer = undefined;
  }

  private reportRenderError(error: GenerativeDomError): void {
    dispatchComponentEvent(
      this,
      "markdown-render-error",
      `${error.phase}:${error.plugin}: ${error.error.message}`,
    );
  }

  private reportMermaidErrors(errors: Error[]): void {
    const firstError = errors[0]?.message ?? "Unknown Mermaid rendering error";
    const remaining = errors.length > 1 ? ` (${errors.length - 1} more)` : "";
    dispatchComponentEvent(this, "markdown-render-error", `mermaid: ${firstError}${remaining}`);
  }

  private handleColorSchemeChange = (): void => {
    if (this.appliedState?.phase === "settled") {
      this.renderSettled(this.appliedState.markdown);
    }
  };
}

if (!customElements.get("lens-markdown")) {
  customElements.define("lens-markdown", StreamingMarkdownElement);
}
