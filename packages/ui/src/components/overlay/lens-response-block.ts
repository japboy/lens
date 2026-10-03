import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { LensOutputBlock, LensResponseBlockDescriptor } from "../../contracts/lens";
import type { LoadResponseBlock, ProvisionalResponseBlock } from "../../contracts/response-history";
import "../../rendering/streaming-markdown";

/** Retains visible Markdown DOM across publications; offscreen bodies are reloadable. */
@customElement("lens-response-block")
export class LensResponseBlock extends LitElement {
  @property({ attribute: false }) operationId = "";
  @property({ attribute: false }) responseId = "";
  @property({ attribute: false }) descriptor: LensResponseBlockDescriptor | undefined;
  @property({ attribute: false }) loadBlock: LoadResponseBlock | undefined;
  @property({ attribute: false }) provisional: ProvisionalResponseBlock | undefined;
  @state() private body: LensOutputBlock | undefined;
  @state() private failure = "";
  private observer: IntersectionObserver | undefined;
  private generation = 0;
  private measuredHeight = 100;
  private resize: ResizeObserver | undefined;
  private visible = false;
  @state() private loading = false;
  @state() private provisionalReleased = false;
  protected createRenderRoot(): HTMLElement {
    return this;
  }
  connectedCallback(): void {
    super.connectedCallback();
    this.style.display = "block";
    this.style.minHeight = `${this.measuredHeight}px`;
    if (typeof ResizeObserver !== "undefined") {
      this.resize = new ResizeObserver((entries) => {
        const height = entries[0]?.contentRect.height;
        if (height && (this.body || (this.provisional && !this.provisionalReleased)))
          this.measuredHeight = height;
      });
      this.resize.observe(this);
    }
    if (typeof IntersectionObserver === "undefined") {
      this.visible = true;
      this.requestUpdate();
      return;
    }
    this.observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.some((entry) => entry.isIntersecting);
        if (visible === this.visible && (visible || !this.provisional || this.provisionalReleased))
          return;
        this.visible = visible;
        if (visible) {
          void this.load();
        } else {
          const height = this.getBoundingClientRect().height;
          if (height > 0) this.measuredHeight = height;
          this.style.minHeight = `${this.measuredHeight}px`;
          ++this.generation;
          this.loading = false;
          this.body = undefined;
          this.failure = "";
          this.releaseProvisional();
        }
      },
      { root: this.closest("[data-auto-scroll-container]"), rootMargin: "500px" },
    );
    this.observer.observe(this);
  }
  disconnectedCallback(): void {
    this.style.minHeight = `${this.measuredHeight}px`;
    this.resize?.disconnect();
    this.resize = undefined;
    super.disconnectedCallback();
    this.observer?.disconnect();
    this.observer = undefined;
    ++this.generation;
    this.loading = false;
    this.body = undefined;
    this.visible = false;
    this.releaseProvisional();
  }
  protected willUpdate(changed: PropertyValues<this>): void {
    if (changed.has("provisional")) this.provisionalReleased = false;
    if (changed.has("operationId") || changed.has("responseId") || changed.has("descriptor")) {
      const old = changed.get("descriptor") as LensResponseBlockDescriptor | undefined;
      if (
        changed.has("operationId") ||
        changed.has("responseId") ||
        old?.block_index !== this.descriptor?.block_index
      ) {
        ++this.generation;
        this.body = undefined;
        this.failure = "";
        this.loading = false;
        this.style.minHeight = "100px";
      }
    }
    if (this.provisional && !this.provisionalReleased) this.style.minHeight = "";
    if (this.visible) void this.load();
    this.setAttribute("aria-busy", String(this.loading));
  }
  private async load(): Promise<void> {
    const descriptor = this.descriptor;
    if (
      !descriptor ||
      descriptor.type === "unsupported" ||
      !this.loadBlock ||
      !this.visible ||
      this.body ||
      this.loading ||
      this.failure
    )
      return;
    const generation = this.generation;
    this.loading = true;
    try {
      const body = await this.loadBlock(this.operationId, this.responseId, descriptor.block_index);
      if (generation !== this.generation || !this.isConnected || !this.visible) return;
      this.body = body;
      this.style.minHeight = "";
      this.releaseProvisional();
    } catch {
      if (generation === this.generation && this.isConnected)
        this.failure = "This response could not be loaded. Scroll back to retry.";
    } finally {
      if (generation === this.generation) this.loading = false;
    }
  }
  /** Explicit navigation waits for its actual body, rather than acknowledging a placeholder. */
  async present(): Promise<boolean> {
    const descriptor = this.descriptor;
    const generation = this.generation;
    if (!descriptor || !this.isConnected) return false;
    this.visible = true;
    if (descriptor.type === "unsupported") {
      this.requestUpdate();
      await this.updateComplete;
      return this.isConnected && generation === this.generation;
    }
    if (!this.loadBlock) return false;
    this.loading = true;
    try {
      const body =
        this.body ??
        (await this.loadBlock(this.operationId, this.responseId, descriptor.block_index));
      if (!this.isConnected || generation !== this.generation) return false;
      this.failure = "";
      this.body = body;
      this.style.minHeight = "";
      this.releaseProvisional();
      await this.updateComplete;
      return (
        this.isConnected &&
        generation === this.generation &&
        Boolean(this.querySelector("lens-markdown"))
      );
    } catch {
      if (generation === this.generation && this.isConnected)
        this.failure = "This response could not be loaded. Try again.";
      return false;
    } finally {
      if (generation === this.generation) this.loading = false;
    }
  }

  private releaseProvisional(): void {
    this.provisionalReleased = true;
    this.provisional?.release();
  }

  protected render() {
    const failure = this.failure
      ? html`<p class="error" role="alert">
          ${this.failure}
          <button
            data-lens-button-role="normal"
            type="button"
            @click=${() => {
              this.failure = "";
              void this.load();
            }}
          >
            Retry
          </button>
        </p>`
      : nothing;
    const block = this.body ?? (!this.provisionalReleased ? this.provisional?.body : undefined);
    if (block?.type === "markdown")
      return html`${failure}<lens-markdown
          class="markdown-body"
          .state=${{ operationId: JSON.stringify([this.operationId, this.responseId, this.descriptor?.block_index]), markdown: block.text, phase: "settled", scrollBehavior: "preserve" }}
        ></lens-markdown>`;
    if (this.descriptor?.type === "unsupported")
      return html`<p class="lens-output-unsupported" role="note">
        This agent output type is not supported yet: <code>${this.descriptor.content_type}</code>
      </p>`;
    return failure;
  }
}
