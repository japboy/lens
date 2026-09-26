import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { repeat } from "lit/directives/repeat.js";
import { keyed } from "lit/directives/keyed.js";
import { prepareHtmlPreview } from "../html-output";
import {
  OUTPUT_MEDIA_DEMAND_EVENT,
  dispatchComponentEvent,
  type OutputMediaDemand,
} from "./events";
import type {
  PresentedOutputImage,
  PresentedOutputHtml,
  PresentedOutputMedia,
} from "../output-media";
import type { HtmlOutputContent } from "../application/html-output-controller";

type PreparedHtml = {
  id: string;
  resourceId: string;
  content: string;
} & (
  | { status: "ready"; preview: ReturnType<typeof prepareHtmlPreview> }
  | { status: "failed"; message: string }
);

type MediaOverlay = "none" | "details";
interface FullscreenSession {
  readonly element: HTMLElement;
  readonly root: Document | ShadowRoot;
  readonly media: PresentedOutputMedia;
}
type FullscreenState =
  | { readonly status: "idle" }
  | {
      readonly status: "entering" | "active" | "exiting" | "cancelled";
      readonly session: FullscreenSession;
    };
type ImageLoadState =
  | { readonly status: "loading" }
  | { readonly status: "failed" }
  | { readonly status: "ready"; readonly width: number; readonly height: number };

interface LoadedImage {
  readonly source: string;
  readonly state: ImageLoadState;
}

@customElement("lens-output-media")
export class LensOutputMedia extends LitElement {
  private preparedHtml = new Map<string, PreparedHtml>();
  private static nextId = 0;
  private readonly detailsId = `lens-output-media-details-${++LensOutputMedia.nextId}`;

  @property({ attribute: false })
  media: readonly PresentedOutputMedia[] = [];

  @property({ attribute: false })
  htmlContent: HtmlOutputContent | undefined;

  /** Response-scoped media IDs prevent equal resource IDs from sharing bodies. */
  @property({ attribute: false })
  htmlContents: ReadonlyMap<string, HtmlOutputContent> | undefined;

  @property({ attribute: false })
  mediaErrors: ReadonlyMap<string, string> = new Map();

  /** One parent-owned Notification presentation, mounted only inside fullscreen. */
  @property({ attribute: false })
  notificationContent: unknown;

  private presentationIdentity: string | undefined;
  private navigationRevision = 0;
  private navigation:
    | { media: PresentedOutputMedia; resolve: (value: boolean) => void }
    | undefined;
  private fullscreenExit:
    | { promise: Promise<boolean>; resolve: (value: boolean) => void }
    | undefined;
  private requestedMediaIdentity: string | undefined;

  @state()
  private renderedHtml = new Map<string, { resourceId: string; content: string }>();

  @state()
  private selectedId: string | undefined;

  @state()
  private overlay: MediaOverlay = "none";

  @state()
  private loadedImages: ReadonlyMap<string, LoadedImage> = new Map();

  @state()
  private fullscreen: FullscreenState = { status: "idle" };

  @state()
  private fullscreenError = "";

  connectedCallback(): void {
    super.connectedCallback();
    this.ownerDocument.addEventListener("fullscreenchange", this.handleFullscreenChange);
    if (this.hasUpdated) this.requestUpdate();
  }

  private resizeObserver: ResizeObserver | undefined;
  private scrollFrame: number | undefined;
  private alignAfterUpdate = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected willUpdate(changed: PropertyValues<this>): void {
    if (changed.has("media")) {
      const previous = changed.get("media") ?? [];
      const retained = this.media.find((item) => item.id === this.selectedId);
      const oldSelected = previous.find((item) => item.id === this.selectedId);
      if (!retained || !this.sameMedia(retained, oldSelected)) {
        this.closeExpanded();
        this.selectedId = this.media[0]?.id;
        this.overlay = "none";
        this.fullscreenError = "";
      }
      this.loadedImages = new Map(
        this.media.flatMap((item) => {
          if (item.kind !== "image") return [];
          const loaded = this.loadedImages.get(item.id);
          return loaded && loaded.source === item.source ? [[item.id, loaded] as const] : [];
        }),
      );
      this.alignAfterUpdate =
        this.media.length !== previous.length ||
        this.media.some((item, index) => !this.sameMedia(item, previous[index]));
    }
    this.prepareHtml();
  }

  /** Bound body demand and HTML preparation to the selection and its neighbors. */
  private get mountedMedia(): readonly PresentedOutputMedia[] {
    const selected = this.selectedIndex;
    return this.media.slice(Math.max(0, selected - 1), selected + 2);
  }

  private contentFor(item: PresentedOutputHtml): HtmlOutputContent | undefined {
    const content = this.htmlContents
      ? this.htmlContents.get(item.id)
      : this.media.filter((media) => media.kind === "html").length === 1
        ? this.htmlContent
        : undefined;
    return content?.resourceId === item.resourceId ? content : undefined;
  }

  protected updated(): void {
    const ids = this.mountedMedia.map((item) => item.id);
    const identity = JSON.stringify(ids);
    if (identity !== this.requestedMediaIdentity) {
      this.requestedMediaIdentity = identity;
      dispatchComponentEvent<OutputMediaDemand>(this, OUTPUT_MEDIA_DEMAND_EVENT, { mediaIds: ids });
    }
    const rail = this.querySelector<HTMLElement>(".output-media-rail");
    if (rail && !this.resizeObserver && typeof ResizeObserver !== "undefined") {
      this.resizeObserver = new ResizeObserver(() => this.alignSelection());
      this.resizeObserver.observe(rail);
    }
    if (this.alignAfterUpdate) {
      this.alignAfterUpdate = false;
      this.alignSelection();
    }
    const presentation = {
      selectedMediaId: this.media[this.selectedIndex]?.id,
      fullscreen: this.fullscreen.status !== "idle",
    };
    const presentationIdentity = JSON.stringify(presentation);
    if (presentationIdentity !== this.presentationIdentity) {
      this.presentationIdentity = presentationIdentity;
      dispatchComponentEvent(this, "lens-media-presentation", presentation);
    }
    this.completeNavigation();
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    ++this.navigationRevision;
    this.navigation?.resolve(false);
    this.navigation = undefined;
    this.fullscreenExit?.resolve(false);
    this.fullscreenExit = undefined;
    this.presentationIdentity = undefined;
    this.resizeObserver?.disconnect();
    this.resizeObserver = undefined;
    if (this.scrollFrame !== undefined) cancelAnimationFrame(this.scrollFrame);
    this.ownerDocument.removeEventListener("fullscreenchange", this.handleFullscreenChange);
    this.closeExpanded();
    this.requestedMediaIdentity = undefined;
  }

  private get selectedIndex(): number {
    return Math.max(
      0,
      this.media.findIndex((item) => item.id === this.selectedId),
    );
  }

  private imageState(item: PresentedOutputImage): ImageLoadState {
    if (this.mediaErrors.has(item.id)) return { status: "failed" };
    const loaded = this.loadedImages.get(item.id);
    return loaded && loaded.source === item.source ? loaded.state : { status: "loading" };
  }

  private sameMedia(item: PresentedOutputMedia, previous?: PresentedOutputMedia): boolean {
    if (!previous || item.id !== previous.id || item.kind !== previous.kind) return false;
    return item.kind === "image"
      ? previous.kind === "image" &&
          (!item.source || !previous.source || item.source === previous.source)
      : previous.kind === "html" && item.resourceId === previous.resourceId;
  }

  private htmlState(item: PresentedOutputHtml): "loading" | "ready" | "failed" {
    const content = this.contentFor(item);
    if (content?.resourceId !== item.resourceId) return "loading";
    if (content.status !== "ready") return content.status;
    if (this.preparedHtml.get(item.id)?.status === "failed") return "failed";
    const rendered = this.renderedHtml.get(item.id);
    return rendered?.resourceId === item.resourceId && rendered.content === content.content
      ? "ready"
      : "loading";
  }

  private prepareHtml(): void {
    const mounted = this.mountedMedia;
    const retained = new Set(mounted.map((item) => item.id));
    for (const id of this.preparedHtml.keys()) if (!retained.has(id)) this.preparedHtml.delete(id);
    for (const id of this.renderedHtml.keys()) {
      if (id !== this.media[this.selectedIndex]?.id) this.renderedHtml.delete(id);
    }
    for (const item of mounted) {
      if (item.kind !== "html") continue;
      const content = this.contentFor(item);
      if (content?.status !== "ready") {
        this.preparedHtml.delete(item.id);
        this.renderedHtml.delete(item.id);
        continue;
      }
      if (this.preparedHtml.get(item.id)?.content === content.content) continue;
      this.renderedHtml.delete(item.id);
      const identity = { id: item.id, resourceId: item.resourceId, content: content.content };
      try {
        this.preparedHtml.set(item.id, {
          ...identity,
          status: "ready",
          preview: prepareHtmlPreview(content.content),
        });
      } catch (error) {
        this.preparedHtml.set(item.id, {
          ...identity,
          status: "failed",
          message: error instanceof Error ? error.message : "HTML could not be displayed.",
        });
      }
    }
  }

  protected render() {
    const item = this.media[this.selectedIndex];
    if (!item) return nothing;
    const count = this.media.length;
    const prepared = this.preparedHtml.get(item.id);
    const ordinal = this.selectedIndex + 1;
    const loaded = item.kind === "image" ? this.imageState(item) : undefined;
    const ready =
      item.kind === "image" ? loaded?.status === "ready" : this.htmlState(item) === "ready";
    const expandedMedia = this.fullscreen.status === "idle" ? item : this.fullscreen.session.media;
    return html`
      <section
        class="output-media-hero"
        aria-label="Interpretation media"
        aria-roledescription=${count > 1 ? "carousel" : "media presentation"}
        data-count=${count}
        @keydown=${this.handleKeyDown}
        @pointerdown=${this.handlePointerDown}
      >
        ${
          this.fullscreenError && this.fullscreen.status === "idle"
            ? html`<p class="output-media-error" role="alert">${this.fullscreenError}</p>`
            : nothing
        }
        <div class="output-media-stage">
          ${
            item.kind === "image" && loaded?.status === "ready"
              ? html`<div class="output-media-ambient" aria-hidden="true">
                  <img src=${item.source} alt="" />
                </div>`
              : nothing
          }
          <div class="output-media-rail" @scroll=${this.handleScroll}>
            ${repeat(
              this.media,
              (media) => media.id,
              (media, index) => this.renderSlide(media, index),
            )}
          </div>
          <div class="output-media-overlay">
            ${
              count > 1
                ? html`<span
                    class="output-media-counter"
                    role="status"
                    aria-live="polite"
                    aria-atomic="true"
                  >
                    <span class="visually-hidden">Media </span>${String(ordinal).padStart(2, "0")}
                    <span aria-hidden="true"> / </span
                    ><span class="visually-hidden"> of </span>${String(count).padStart(2, "0")}
                  </span>`
                : nothing
            }
            <div class="output-media-tools">
              <button
                type="button"
                class="output-media-tool output-media-details-toggle"
                aria-label="Media details"
                title="Details"
                aria-expanded=${this.overlay === "details" ? "true" : "false"}
                aria-controls=${this.detailsId}
                @click=${() => {
                  this.overlay = this.overlay === "details" ? "none" : "details";
                }}
              >
                <i class="fa-solid fa-circle-info" aria-hidden="true"></i>
              </button>
              <button
                type="button"
                class="output-media-tool output-media-expand"
                aria-label="Expand media"
                title="View fullscreen"
                ?disabled=${!ready || this.fullscreen.status !== "idle"}
                @click=${this.expandMedia}
              >
                <i class="fa-solid fa-expand" aria-hidden="true"></i>
              </button>
            </div>
          </div>
          ${
            count > 1
              ? html` <button
                    type="button"
                    class="output-media-arrow output-media-previous"
                    aria-label="Previous media"
                    title="Previous media"
                    ?disabled=${this.selectedIndex === 0}
                    @click=${() => this.select(this.selectedIndex - 1)}
                  >
                    <i class="fa-solid fa-arrow-left" aria-hidden="true"></i>
                  </button>
                  <button
                    type="button"
                    class="output-media-arrow output-media-next"
                    aria-label="Next media"
                    title="Next media"
                    ?disabled=${this.selectedIndex === count - 1}
                    @click=${() => this.select(this.selectedIndex + 1)}
                  >
                    <i class="fa-solid fa-arrow-right" aria-hidden="true"></i>
                  </button>`
              : nothing
          }
          ${
            item.kind === "html" && this.overlay === "details"
              ? html`<div class="output-media-details-backdrop" aria-hidden="true"></div>`
              : nothing
          }
          <section
            id=${this.detailsId}
            class="output-media-details"
            aria-label="Media details"
            ?hidden=${this.overlay !== "details"}
          >
            <h2>Media details</h2>
            <dl>
              <dt>Declared format</dt>
              <dd>${item.mimeType}</dd>
              ${
                loaded?.status === "ready"
                  ? html`
                      <dt>Intrinsic size</dt>
                      <dd>${loaded.width} × ${loaded.height} CSS px</dd>
                      <dt>Orientation</dt>
                      <dd>
                        ${loaded.width === loaded.height ? "Square" : loaded.width > loaded.height ? "Landscape" : "Portrait"}
                      </dd>
                    `
                  : nothing
              }
              ${
                item.kind === "html"
                  ? html`<dt>Content size</dt>
                      <dd>${item.byteLength.toLocaleString()} bytes</dd>`
                  : nothing
              }
            </dl>
            ${
              item.kind === "html" && prepared?.status === "ready"
                ? html`${prepared.preview.notices.map((notice) => html`<p>${notice}</p>`)}`
                : nothing
            }
          </section>
        </div>
        <div
          class="output-media-expanded"
          role="dialog"
          aria-modal="true"
          aria-label="Expanded media"
          @keydown=${this.handleExpandedKeyDown}
        >
          <header>
            <span>Media ${ordinal} of ${count}</span>
            <button
              type="button"
              class="output-media-tool output-media-expanded-close"
              aria-label="Close expanded media"
              title="Close expanded media"
              ?disabled=${this.fullscreen.status === "exiting"}
              @click=${this.closeExpanded}
            >
              <i class="fa-solid fa-xmark" aria-hidden="true"></i>
            </button>
          </header>
          ${this.fullscreen.status !== "idle" && this.fullscreen.session.media.kind === "image" ? this.notificationContent : nothing}
          ${this.fullscreenError ? html`<p class="output-media-error" role="alert">${this.fullscreenError}</p>` : nothing}
          ${expandedMedia.kind === "image" && expandedMedia.source ? html`<img src=${expandedMedia.source} alt="Agent image ${ordinal} of ${count}" />` : nothing}
        </div>
      </section>
    `;
  }

  private renderSlide(item: PresentedOutputMedia, index: number) {
    if (item.kind === "html") return this.renderHtmlSlide(item, index);
    const mounted = Math.abs(index - this.selectedIndex) <= 1;
    const loaded = this.imageState(item);
    return html`<figure
      class="output-media-slide"
      aria-roledescription="slide"
      aria-label="Media ${index + 1} of ${this.media.length}"
      aria-hidden=${index !== this.selectedIndex ? "true" : "false"}
      ?inert=${index !== this.selectedIndex}
      data-load-state=${loaded.status}
    >
      ${
        mounted && item.source
          ? html`<img
              src=${item.source}
              alt="Agent image ${index + 1} of ${this.media.length}"
              loading=${index === this.selectedIndex ? "eager" : "lazy"}
              decoding="async"
              draggable="false"
              @load=${(event: Event) => this.handleImageLoad(item, event)}
              @error=${(event: Event) => this.handleImageError(item, event)}
            />`
          : nothing
      }
      ${
        mounted && loaded.status !== "ready"
          ? html`<p class="output-media-state" role="status">
              ${loaded.status === "failed" ? (this.mediaErrors.get(item.id) ?? "Unable to display this image.") : "Loading image…"}
            </p>`
          : nothing
      }
    </figure>`;
  }

  private renderHtmlSlide(item: PresentedOutputHtml, index: number) {
    const content = this.contentFor(item);
    const prepared = this.preparedHtml.get(item.id);
    const selected = index === this.selectedIndex;
    const message =
      content?.status === "failed"
        ? content.message
        : prepared?.status === "failed"
          ? prepared.message
          : this.htmlState(item) === "loading"
            ? "Loading HTML…"
            : "";
    return html`<section
      class="output-media-slide output-media-html-slide"
      aria-roledescription="slide"
      aria-label="HTML ${index + 1} of ${this.media.length}"
      aria-hidden=${index !== this.selectedIndex ? "true" : "false"}
      ?inert=${index !== this.selectedIndex}
      @keydown=${this.handleExpandedKeyDown}
    >
      <div class="output-media-html-content">
        <header class="output-html-expanded-header">
          <span>Media ${index + 1} of ${this.media.length}</span>
          <button
            type="button"
            class="output-media-tool output-html-expanded-close"
            aria-label="Close expanded HTML"
            title="Close expanded media"
            ?disabled=${this.fullscreen.status === "exiting"}
            @click=${this.closeExpanded}
          >
            <i class="fa-solid fa-xmark" aria-hidden="true"></i>
          </button>
        </header>
        ${selected && this.fullscreen.status !== "idle" && this.fullscreen.session.media.kind === "html" ? this.notificationContent : nothing}
        ${this.fullscreenError && this.fullscreen.status !== "idle" ? html`<p class="output-media-error" role="alert">${this.fullscreenError}</p>` : nothing}
        ${
          // WebKit can retain stale iframe hit-testing after an inert ancestor is
          // re-enabled. Only create an iframe under the selected, interactive slide.
          selected && prepared?.status === "ready"
            ? keyed(
                prepared,
                html`<iframe
                  class="output-html-frame"
                  title="HTML content"
                  sandbox="allow-popups"
                  referrerpolicy="no-referrer"
                  .srcdoc=${prepared.preview.document}
                  @load=${(event: Event) => this.handleHtmlLoad(prepared, event)}
                ></iframe>`,
              )
            : nothing
        }
        ${selected && message ? html`<p class="output-media-state" role="status">${message}</p>` : nothing}
      </div>
    </section>`;
  }

  private handleHtmlLoad(prepared: PreparedHtml, event: Event): void {
    const frame = event.currentTarget as HTMLIFrameElement;
    const item = this.media.find(
      (media): media is PresentedOutputHtml => media.kind === "html" && media.id === prepared.id,
    );
    const content = item ? this.contentFor(item) : undefined;
    if (
      prepared !== this.preparedHtml.get(prepared.id) ||
      prepared.status !== "ready" ||
      !frame.isConnected ||
      !this.contains(frame) ||
      frame.srcdoc !== prepared.preview.document ||
      content?.resourceId !== prepared.resourceId ||
      content.status !== "ready" ||
      content.content !== prepared.content
    )
      return;
    this.renderedHtml = new Map(this.renderedHtml).set(prepared.id, {
      resourceId: content.resourceId,
      content: content.content,
    });
  }

  private handleImageLoad(item: PresentedOutputImage, event: Event): void {
    const image = event.currentTarget as HTMLImageElement;
    if (
      !item.source ||
      !image.isConnected ||
      (image.currentSrc && image.currentSrc !== item.source)
    )
      return;
    if (!this.media.some((current) => this.sameMedia(current, item))) return;
    const state: ImageLoadState =
      image.naturalWidth > 0 && image.naturalHeight > 0
        ? { status: "ready", width: image.naturalWidth, height: image.naturalHeight }
        : { status: "failed" };
    this.loadedImages = new Map(this.loadedImages).set(item.id, { source: item.source, state });
  }

  private handleImageError(item: PresentedOutputImage, event: Event): void {
    const image = event.currentTarget as HTMLImageElement;
    if (!item.source || !image.isConnected || image.getAttribute("src") !== item.source) return;
    if (!this.media.some((current) => this.sameMedia(current, item))) return;
    this.loadedImages = new Map(this.loadedImages).set(item.id, {
      source: item.source,
      state: { status: "failed" },
    });
  }

  /** Explicit navigation only; body demand and ordinary arrivals never acknowledge it. */
  async presentMedia(mediaId: string): Promise<boolean> {
    const revision = ++this.navigationRevision;
    this.navigation?.resolve(false);
    this.navigation = undefined;
    const media = this.media.find((item) => item.id === mediaId);
    if (!media || !this.isConnected || !(await this.exitFullscreen())) return false;
    if (
      revision !== this.navigationRevision ||
      !this.isConnected ||
      !this.media.some((item) => this.sameMedia(item, media))
    )
      return false;
    return new Promise<boolean>((resolve) => {
      this.navigation = { media, resolve };
      this.selectedId = mediaId;
      this.overlay = "none";
      this.alignAfterUpdate = true;
      this.requestUpdate();
    });
  }

  /** Wait for owned fullscreen to leave before a parent navigates to narrative. */
  exitFullscreen(): Promise<boolean> {
    if (!this.isConnected) return Promise.resolve(false);
    if (this.fullscreen.status === "idle") return Promise.resolve(true);
    if (this.fullscreenExit) return this.fullscreenExit.promise;
    let resolve!: (value: boolean) => void;
    const promise = new Promise<boolean>((done) => {
      resolve = done;
    });
    this.fullscreenExit = { promise, resolve };
    this.closeExpanded();
    return promise;
  }

  private completeNavigation(): void {
    const pending = this.navigation;
    if (!pending) return;
    const item = this.media[this.selectedIndex];
    const state =
      item?.kind === "image"
        ? this.imageState(item).status
        : item?.kind === "html"
          ? this.htmlState(item)
          : "failed";
    if (!item || !this.sameMedia(item, pending.media) || state === "failed") {
      this.navigation = undefined;
      pending.resolve(false);
    } else if (state === "ready" && this.fullscreen.status === "idle") {
      const rail = this.querySelector<HTMLElement>(".output-media-rail");
      if (
        !rail ||
        Math.abs((this.slideOffsets(rail)[this.selectedIndex] ?? 0) - rail.scrollLeft) > 2
      )
        return;
      this.navigation = undefined;
      pending.resolve(true);
    }
  }

  private select(index: number): void {
    if (this.fullscreen.status !== "idle") return;
    const next = this.media[Math.max(0, Math.min(this.media.length - 1, index))];
    if (!next) return;
    const focused = (this.getRootNode() as Document | ShadowRoot).activeElement;
    this.selectedId = next.id;
    this.overlay = "none";
    void this.updateComplete.then(() => {
      const previous = this.querySelector<HTMLButtonElement>(".output-media-previous");
      const nextButton = this.querySelector<HTMLButtonElement>(".output-media-next");
      if (focused === nextButton && nextButton?.disabled) previous?.focus({ preventScroll: true });
      if (focused === previous && previous?.disabled) nextButton?.focus({ preventScroll: true });
    });
    const rail = this.querySelector<HTMLElement>(".output-media-rail");
    rail?.scrollTo({
      left: this.slideOffsets(rail)[this.selectedIndex] ?? 0,
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
        ? "instant"
        : "smooth",
    });
  }

  /** DOM geometry retains subpixel positions; clientWidth rounds each slide's width. */
  private slideOffsets(rail: HTMLElement): number[] {
    const frame = rail.getBoundingClientRect();
    return [...rail.children].map((slide, index) => {
      const bounds = slide.getBoundingClientRect();
      return bounds.width > 0
        ? rail.scrollLeft + bounds.left - frame.left - rail.clientLeft
        : index * rail.clientWidth;
    });
  }

  private alignSelection(): void {
    const rail = this.querySelector<HTMLElement>(".output-media-rail");
    if (rail) rail.scrollLeft = this.slideOffsets(rail)[this.selectedIndex] ?? 0;
  }

  private handleScroll = (): void => {
    // Ignore fullscreen layout events at receipt, before a queued RAF can run
    // after fullscreenchange has returned the state to idle.
    if (this.fullscreen.status !== "idle") return;
    if (this.scrollFrame !== undefined) cancelAnimationFrame(this.scrollFrame);
    this.scrollFrame = requestAnimationFrame(() => {
      if (this.fullscreen.status !== "idle") return;
      const rail = this.querySelector<HTMLElement>(".output-media-rail");
      if (!rail?.clientWidth) return;
      const offsets = this.slideOffsets(rail);
      let index = 0;
      for (let candidate = 1; candidate < offsets.length; candidate++) {
        if (
          Math.abs(offsets[candidate]! - rail.scrollLeft) <
          Math.abs(offsets[index]! - rail.scrollLeft)
        )
          index = candidate;
      }
      const next = this.media[index];
      if (next && next.id !== this.selectedId) {
        this.selectedId = next.id;
        this.overlay = "none";
      }
    });
  };

  private handleKeyDown = (event: KeyboardEvent): void => {
    if (this.fullscreen.status !== "idle") return;
    if (
      event
        .composedPath()
        .some((node) => node instanceof HTMLElement && node.localName === "lens-html-output")
    )
      return;
    if (event.key === "Escape" && this.overlay === "details") {
      event.preventDefault();
      this.overlay = "none";
      this.querySelector<HTMLButtonElement>(".output-media-details-toggle")?.focus({
        preventScroll: true,
      });
      return;
    }
    const index = (() => {
      switch (event.key) {
        case "ArrowLeft":
          return this.selectedIndex - 1;
        case "ArrowRight":
          return this.selectedIndex + 1;
        case "Home":
          return 0;
        case "End":
          return this.media.length - 1;
        default:
          return undefined;
      }
    })();
    if (index === undefined) return;
    event.preventDefault();
    this.select(index);
  };

  private handlePointerDown = (event: PointerEvent): void => {
    if (
      this.overlay === "details" &&
      !(event.target as Element).closest(".output-media-details, .output-media-details-toggle")
    ) {
      this.overlay = "none";
    }
  };

  private ownsFullscreen(session: FullscreenSession): boolean {
    return session.root.fullscreenElement === session.element;
  }

  private finishFullscreen(session: FullscreenSession): void {
    if (this.fullscreen.status === "idle" || this.fullscreen.session !== session) return;
    if (this.scrollFrame !== undefined) cancelAnimationFrame(this.scrollFrame);
    this.fullscreen = { status: "idle" };
    this.fullscreenExit?.resolve(this.isConnected);
    this.fullscreenExit = undefined;
    if (this.isConnected) {
      void this.updateComplete.then(() => {
        if (this.isConnected && this.fullscreen.status === "idle") {
          this.querySelector<HTMLButtonElement>(".output-media-expand")?.focus({
            preventScroll: true,
          });
        }
      });
    }
  }

  private expandMedia = async (): Promise<void> => {
    if (this.fullscreen.status !== "idle") return;
    const media = this.media[this.selectedIndex];
    const element =
      media?.kind === "html"
        ? this.querySelector<HTMLElement>(
            '.output-media-html-slide[aria-hidden="false"] .output-media-html-content',
          )
        : this.querySelector<HTMLElement>(".output-media-expanded");
    if (
      !element ||
      !media ||
      (media.kind === "image" ? this.imageState(media).status : this.htmlState(media)) !== "ready"
    )
      return;
    this.overlay = "none";
    this.fullscreenError = "";
    if (
      typeof element.requestFullscreen !== "function" ||
      this.ownerDocument.fullscreenEnabled === false
    ) {
      this.fullscreenError = "Fullscreen is unavailable. You can try again.";
      return;
    }
    const session: FullscreenSession = {
      element,
      root: element.getRootNode() as Document | ShadowRoot,
      media,
    };
    this.fullscreen = { status: "entering", session };
    try {
      // Request directly in the click handler while transient user activation is available.
      await element.requestFullscreen();
      this.reconcileFullscreen(session);
    } catch {
      this.failFullscreen(session);
    }
  };

  private failFullscreen(session: FullscreenSession): void {
    if (this.fullscreen.status === "idle" || this.fullscreen.session !== session) return;
    if (this.fullscreen.status !== "cancelled" && this.isConnected) {
      this.fullscreenError = "Unable to open fullscreen. Please try again.";
    }
    this.finishFullscreen(session);
  }

  private reconcileFullscreen(session: FullscreenSession): void {
    if (this.fullscreen.status === "idle" || this.fullscreen.session !== session) return;
    if (!this.ownsFullscreen(session)) {
      this.finishFullscreen(session);
      return;
    }
    if (this.fullscreen.status === "cancelled" || !this.isConnected) {
      this.closeExpanded();
    } else if (this.fullscreen.status === "entering") {
      this.fullscreen = { status: "active", session };
      void this.updateComplete.then(() => {
        if (this.fullscreen.status === "active" && this.fullscreen.session === session) {
          session.element
            .querySelector<HTMLButtonElement>("button")
            ?.focus({ preventScroll: true });
        }
      });
    }
  }

  private handleFullscreenChange = (): void => {
    if (this.fullscreen.status === "idle") return;
    const { session, status } = this.fullscreen;
    // Unrelated fullscreen events must not cancel an outstanding entry request.
    if (this.ownsFullscreen(session) || status === "active" || status === "exiting") {
      this.reconcileFullscreen(session);
    }
  };

  private closeExpanded = (): void => {
    if (this.fullscreen.status === "idle" || this.fullscreen.status === "exiting") return;
    const { session } = this.fullscreen;
    if (!this.ownsFullscreen(session)) {
      if (this.fullscreen.status === "entering" || this.fullscreen.status === "cancelled") {
        this.fullscreen = { status: "cancelled", session };
      } else {
        this.finishFullscreen(session);
      }
      return;
    }
    this.fullscreen = { status: "exiting", session };
    this.fullscreenError = "";
    void this.ownerDocument.exitFullscreen().then(
      () => this.reconcileFullscreen(session),
      () => {
        if (this.fullscreen.status === "idle" || this.fullscreen.session !== session) return;
        if (this.ownsFullscreen(session)) {
          this.fullscreen = { status: "active", session };
          this.fullscreenError = "Unable to leave fullscreen. Press Escape or try again.";
          this.fullscreenExit?.resolve(false);
          this.fullscreenExit = undefined;
        } else {
          this.finishFullscreen(session);
        }
      },
    );
  };

  /** Traverse the rendered tree so form controls inside open shadow roots remain reachable. */
  private fullscreenTabStops(root: HTMLElement): HTMLElement[] {
    const controls: HTMLElement[] = [];
    const visit = (element: Element): void => {
      if (!(element instanceof HTMLElement)) return;
      const style = this.ownerDocument.defaultView?.getComputedStyle(element);
      if (
        element.matches('[hidden], [inert], [aria-hidden="true"]') ||
        style?.display === "none" ||
        style?.visibility === "hidden"
      )
        return;
      if (element.tabIndex >= 0 && !element.matches(":disabled")) controls.push(element);
      const children =
        element instanceof HTMLSlotElement
          ? element.assignedElements({ flatten: true })
          : [...(element.shadowRoot?.children ?? element.children)];
      for (const child of children) visit(child);
    };
    for (const child of root.children) visit(child);
    return controls;
  }

  private handleExpandedKeyDown = (event: KeyboardEvent): void => {
    if (this.fullscreen.status === "idle") return;
    event.stopPropagation();
    if (event.defaultPrevented) return;
    if (event.key === "Escape") {
      event.preventDefault();
      this.closeExpanded();
    } else if (event.key === "Tab" && this.fullscreen.session.media.kind === "image") {
      event.preventDefault();
      const controls = this.fullscreenTabStops(this.fullscreen.session.element);
      // composedPath starts at the actual control, unlike retargeted activeElement.
      const active = event.composedPath()[0];
      const index = controls.findIndex((control) => control === active);
      const next =
        index < 0
          ? event.shiftKey
            ? controls.length - 1
            : 0
          : (index + (event.shiftKey ? -1 : 1) + controls.length) % controls.length;
      controls[next]?.focus({ preventScroll: true });
    }
  };
}
