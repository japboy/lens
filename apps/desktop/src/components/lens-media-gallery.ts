import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { LensState } from "../types";
import { inputMediaPreviewUrl } from "../view-model";

@customElement("lens-media-gallery")
export class LensMediaGallery extends LitElement {
  @property({ attribute: false })
  lens: LensState = { stage: "idle", prompt_execution_revision: 1, output_blocks: [] };

  @state()
  private activeAttachmentId: string | undefined;

  @state()
  private previewError = "";

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("lens")) return;
    const previous = changed.get("lens");
    const media = this.lens.input?.media ?? [];
    const selectedStillExists = media.some(({ id }) => id === this.activeAttachmentId);
    const nextAttachmentId =
      previous?.operation_id !== this.lens.operation_id || !selectedStillExists
        ? media[0]?.id
        : this.activeAttachmentId;
    const previousUri = previous?.input?.media.find(
      ({ id }) => id === this.activeAttachmentId,
    )?.uri;
    const nextUri = media.find(({ id }) => id === nextAttachmentId)?.uri;
    if (nextAttachmentId !== this.activeAttachmentId || previousUri !== nextUri) {
      this.activeAttachmentId = nextAttachmentId;
      this.previewError = "";
    }
  }

  protected render() {
    const media = this.lens.input?.media ?? [];
    if (!media.length) return nothing;
    const selectedIndex = media.findIndex(({ id }) => id === this.activeAttachmentId);
    const index = selectedIndex >= 0 ? selectedIndex : 0;
    const attachment = media[index];
    if (!attachment) return nothing;
    const source = inputMediaPreviewUrl(this.lens, attachment);
    const scopeLabel =
      attachment.scope === "window_fallback" ? "Whole-window fallback" : "AX image region";
    const alt =
      attachment.scope === "window_fallback"
        ? "Whole-window fallback sent to the Agent"
        : `AX image region sent to the Agent for node ${attachment.source_node_id ?? "unknown"}`;

    return html`
      <section class="input-media-preview" aria-labelledby="input-media-heading">
        <header>
          <h2 id="input-media-heading">Input images</h2>
          <span>${index + 1} of ${media.length}</span>
        </header>
        <div class="input-media-carousel" role="group" aria-label="Input image carousel">
          <button
            type="button"
            aria-label="Previous input image"
            ?disabled=${index === 0}
            @click=${() => this.select(media[index - 1]?.id)}
          >
            <span aria-hidden="true">‹</span>
          </button>
          <ol class="input-media-thumbnails" aria-label="Input image thumbnails">
            ${media.map(
              (candidate, candidateIndex) => html`
                <li>
                  <button
                    type="button"
                    class=${
                      candidateIndex === index
                        ? "input-media-thumbnail is-selected"
                        : "input-media-thumbnail"
                    }
                    aria-label=${`Show input image ${candidateIndex + 1} of ${media.length}`}
                    aria-current=${candidateIndex === index ? "true" : "false"}
                    @click=${() => this.select(candidate.id)}
                  >
                    <img
                      src=${inputMediaPreviewUrl(this.lens, candidate) ?? ""}
                      alt=""
                      draggable="false"
                    />
                    <span aria-hidden="true">${candidateIndex + 1}</span>
                  </button>
                </li>
              `,
            )}
          </ol>
          <button
            type="button"
            aria-label="Next input image"
            ?disabled=${index === media.length - 1}
            @click=${() => this.select(media[index + 1]?.id)}
          >
            <span aria-hidden="true">›</span>
          </button>
        </div>
        <figure>
          ${
            source
              ? html`<img
                  src=${source}
                  alt=${alt}
                  draggable="false"
                  ?hidden=${Boolean(this.previewError)}
                  @load=${() => {
                    if (this.activeAttachmentId === attachment.id) this.previewError = "";
                  }}
                  @error=${() => {
                    if (this.activeAttachmentId === attachment.id) {
                      this.previewError =
                        "The selected input image is no longer available for this operation.";
                    }
                  }}
                />`
              : nothing
          }
          ${
            !source || this.previewError
              ? html`<p class="input-media-error" role="alert">
                  ${
                    this.previewError ||
                    "The selected input image URI does not match the current operation."
                  }
                </p>`
              : nothing
          }
          <figcaption>
            <dl class="input-media-metadata">
              <div>
                <dt>Scope</dt>
                <dd>${scopeLabel}</dd>
              </div>
              <div>
                <dt>Attachment</dt>
                <dd><code>${attachment.id}</code></dd>
              </div>
              <div>
                <dt>Target</dt>
                <dd><code>${attachment.target_id}</code></dd>
              </div>
              <div>
                <dt>AX node</dt>
                <dd>
                  ${attachment.source_node_id ? html`<code>${attachment.source_node_id}</code>` : "—"}
                </dd>
              </div>
              <div>
                <dt>Pixels</dt>
                <dd>${attachment.pixel_width} × ${attachment.pixel_height}</dd>
              </div>
              <div>
                <dt>Coverage</dt>
                <dd>${attachment.coverage}</dd>
              </div>
              <div>
                <dt>Source bounds</dt>
                <dd>
                  ${attachment.source_bounds.x}, ${attachment.source_bounds.y} ·
                  ${attachment.source_bounds.width} × ${attachment.source_bounds.height}
                </dd>
              </div>
              <div>
                <dt>Captured bounds</dt>
                <dd>
                  ${attachment.captured_bounds.x}, ${attachment.captured_bounds.y} ·
                  ${attachment.captured_bounds.width} × ${attachment.captured_bounds.height}
                </dd>
              </div>
              <div>
                <dt>Coordinates</dt>
                <dd>${attachment.coordinate_space}</dd>
              </div>
              <div>
                <dt>Format</dt>
                <dd>${attachment.mime_type}</dd>
              </div>
              <div>
                <dt>Encoded size</dt>
                <dd>${attachment.encoded_bytes.toLocaleString()} bytes</dd>
              </div>
            </dl>
          </figcaption>
        </figure>
      </section>
    `;
  }

  private select(attachmentId: string | undefined): void {
    if (!attachmentId || !this.lens.input?.media.some(({ id }) => id === attachmentId)) return;
    this.activeAttachmentId = attachmentId;
    this.previewError = "";
  }
}
