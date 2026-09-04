import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { repeat } from "lit/directives/repeat.js";
import type { PresentedOutputImage } from "../output-media";

type MediaOverlay = "none" | "details" | "expanded";
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
  private static nextId = 0;
  private readonly detailsId = `lens-output-media-details-${++LensOutputMedia.nextId}`;

  @property({ attribute: false })
  media: readonly PresentedOutputImage[] = [];

  @state()
  private selectedId: string | undefined;

  @state()
  private overlay: MediaOverlay = "none";

  @state()
  private loadedImages: ReadonlyMap<string, LoadedImage> = new Map();

  private resizeObserver: ResizeObserver | undefined;
  private scrollFrame: number | undefined;
  private alignAfterUpdate = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("media")) return;
    const previous = changed.get("media") ?? [];
    if (
      this.media.length === previous.length &&
      this.media.every(
        (item, index) => item.id === previous[index]?.id && item.source === previous[index]?.source,
      )
    )
      return;
    const retained = this.media.find((item) => item.id === this.selectedId);
    const oldSelected = previous.find((item) => item.id === this.selectedId);
    if (!retained || retained.source !== oldSelected?.source) {
      this.selectedId = this.media[0]?.id;
      this.overlay = "none";
    }
    this.loadedImages = new Map(
      this.media.flatMap((item) => {
        const loaded = this.loadedImages.get(item.id);
        return loaded?.source === item.source ? [[item.id, loaded] as const] : [];
      }),
    );
    this.alignAfterUpdate = true;
  }

  protected updated(): void {
    const rail = this.querySelector<HTMLElement>(".output-media-rail");
    if (rail && !this.resizeObserver && typeof ResizeObserver !== "undefined") {
      this.resizeObserver = new ResizeObserver(() => this.alignSelection());
      this.resizeObserver.observe(rail);
    }
    if (this.alignAfterUpdate) {
      this.alignAfterUpdate = false;
      this.alignSelection();
    }
    const dialog = this.querySelector<HTMLDialogElement>(".output-media-expanded");
    if (dialog) {
      if (this.overlay === "expanded" && !dialog.open) dialog.showModal();
      if (this.overlay !== "expanded" && dialog.open) dialog.close();
    }
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.resizeObserver?.disconnect();
    this.resizeObserver = undefined;
    if (this.scrollFrame !== undefined) cancelAnimationFrame(this.scrollFrame);
    const dialog = this.querySelector<HTMLDialogElement>("dialog");
    if (dialog?.open) dialog.close();
  }

  private get selectedIndex(): number {
    return Math.max(
      0,
      this.media.findIndex((item) => item.id === this.selectedId),
    );
  }

  private imageState(item: PresentedOutputImage): ImageLoadState {
    const loaded = this.loadedImages.get(item.id);
    return loaded?.source === item.source ? loaded.state : { status: "loading" };
  }

  protected render() {
    const item = this.media[this.selectedIndex];
    if (!item) return nothing;
    const count = this.media.length;
    const ordinal = this.selectedIndex + 1;
    const loaded = this.imageState(item);
    return html`
      <section
        class="output-media-hero"
        aria-label="Interpretation media"
        aria-roledescription=${count > 1 ? "carousel" : "image presentation"}
        data-count=${count}
        @keydown=${this.handleKeyDown}
        @pointerdown=${this.handlePointerDown}
      >
        ${
          loaded.status === "ready"
            ? html`<div class="output-media-ambient" aria-hidden="true">
                <img src=${item.source} alt="" />
              </div>`
            : nothing
        }
        <div class="output-media-rail" @scroll=${this.handleScroll}>
          ${repeat(
            this.media,
            (image) => image.id,
            (image, index) => this.renderSlide(image, index),
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
              title="Expand"
              ?disabled=${loaded.status !== "ready"}
              @click=${() => {
                this.overlay = "expanded";
              }}
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
              loaded.status === "ready"
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
          </dl>
        </section>
        <dialog
          class="output-media-expanded"
          aria-label="Expanded media"
          @cancel=${this.closeExpanded}
          @close=${this.handleDialogClose}
        >
          <header>
            <span>Media ${ordinal} of ${count}</span>
            <button
              type="button"
              class="output-media-tool"
              aria-label="Close expanded media"
              title="Close expanded media"
              autofocus
              @click=${this.closeExpanded}
            >
              <i class="fa-solid fa-xmark" aria-hidden="true"></i>
            </button>
          </header>
          ${
            this.overlay === "expanded"
              ? html`<img src=${item.source} alt="Agent image ${ordinal} of ${count}" />`
              : nothing
          }
        </dialog>
      </section>
    `;
  }

  private renderSlide(item: PresentedOutputImage, index: number) {
    const loaded = this.imageState(item);
    return html`<figure
      class="output-media-slide"
      aria-roledescription="slide"
      aria-label="Media ${index + 1} of ${this.media.length}"
      aria-hidden=${index !== this.selectedIndex ? "true" : "false"}
      data-load-state=${loaded.status}
    >
      <img
        src=${item.source}
        alt="Agent image ${index + 1} of ${this.media.length}"
        loading=${index === this.selectedIndex ? "eager" : "lazy"}
        decoding="async"
        draggable="false"
        @load=${(event: Event) => this.handleImageLoad(item, event)}
        @error=${(event: Event) => this.handleImageError(item, event)}
      />
      ${
        loaded.status !== "ready"
          ? html`<p class="output-media-state" role="status">
              ${loaded.status === "failed" ? "Unable to display this image." : "Loading image…"}
            </p>`
          : nothing
      }
    </figure>`;
  }

  private handleImageLoad(item: PresentedOutputImage, event: Event): void {
    const image = event.currentTarget as HTMLImageElement;
    if (image.currentSrc && image.currentSrc !== item.source) return;
    if (!this.media.some((current) => current.id === item.id && current.source === item.source))
      return;
    const state: ImageLoadState =
      image.naturalWidth > 0 && image.naturalHeight > 0
        ? { status: "ready", width: image.naturalWidth, height: image.naturalHeight }
        : { status: "failed" };
    this.loadedImages = new Map(this.loadedImages).set(item.id, { source: item.source, state });
  }

  private handleImageError(item: PresentedOutputImage, event: Event): void {
    const image = event.currentTarget as HTMLImageElement;
    if (image.getAttribute("src") !== item.source) return;
    if (!this.media.some((current) => current.id === item.id && current.source === item.source))
      return;
    this.loadedImages = new Map(this.loadedImages).set(item.id, {
      source: item.source,
      state: { status: "failed" },
    });
  }

  private select(index: number): void {
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
      left: this.selectedIndex * rail.clientWidth,
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
        ? "instant"
        : "smooth",
    });
  }

  private alignSelection(): void {
    const rail = this.querySelector<HTMLElement>(".output-media-rail");
    if (rail) rail.scrollLeft = this.selectedIndex * rail.clientWidth;
  }

  private handleScroll = (): void => {
    if (this.scrollFrame !== undefined) cancelAnimationFrame(this.scrollFrame);
    this.scrollFrame = requestAnimationFrame(() => {
      const rail = this.querySelector<HTMLElement>(".output-media-rail");
      if (!rail?.clientWidth) return;
      const index = Math.round(rail.scrollLeft / rail.clientWidth);
      if (Math.abs(rail.scrollLeft - index * rail.clientWidth) > 1) return;
      const next = this.media[index];
      if (next && next.id !== this.selectedId) {
        this.selectedId = next.id;
        this.overlay = "none";
      }
    });
  };

  private handleKeyDown = (event: KeyboardEvent): void => {
    if (this.overlay === "expanded") return;
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

  private closeExpanded = (event: Event): void => {
    event.preventDefault();
    this.overlay = "none";
  };

  private handleDialogClose = (): void => {
    this.overlay = "none";
    this.querySelector<HTMLButtonElement>(".output-media-expand")?.focus({ preventScroll: true });
  };
}
