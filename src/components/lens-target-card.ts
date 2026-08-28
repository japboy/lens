import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { LensTargetSelectionItem } from "../types";
import { dispatchComponentEvent, TARGET_REMOVE_EVENT } from "./events";

@customElement("lens-target-card")
export class LensTargetCard extends LitElement {
  @property({ attribute: false })
  item: LensTargetSelectionItem | undefined;

  @property({ type: Boolean })
  disabled = false;

  @state()
  private previewFailed = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("item")) return;
    const previous = changed.get("item");
    if (previous?.id !== this.item?.id || previous?.preview_uri !== this.item?.preview_uri) {
      this.previewFailed = false;
    }
  }

  protected render() {
    const item = this.item;
    if (!item) return nothing;
    const label = `${item.window.application_name}${
      item.window.title ? ` — ${item.window.title}` : ""
    }`;
    return html`
      <div class="target-selection-image">
        <div class="target-selection-placeholder" aria-hidden="true">
          <i class="fa-solid fa-window-maximize"></i>
        </div>
        ${
          item.preview_uri && !this.previewFailed
            ? html`<img
                src=${item.preview_uri}
                alt=${`Preview of ${label}`}
                draggable="false"
                @error=${() => {
                  this.previewFailed = true;
                }}
              />`
            : nothing
        }
        <button
          type="button"
          class="target-selection-remove"
          aria-label=${`Remove ${label}`}
          title="Remove window"
          ?disabled=${this.disabled}
          @click=${() => dispatchComponentEvent(this, TARGET_REMOVE_EVENT, { targetId: item.id })}
        >
          <i class="fa-solid fa-xmark" aria-hidden="true"></i>
        </button>
      </div>
      <div class="target-selection-caption">
        <strong>${item.window.application_name || "Application"}</strong>
        <span title=${item.window.title || "Untitled window"}
          >${item.window.title || "Untitled window"}</span
        >
      </div>
      ${
        item.preview_error || this.previewFailed
          ? html`<span class="visually-hidden">Preview unavailable</span>`
          : nothing
      }
    `;
  }
}
