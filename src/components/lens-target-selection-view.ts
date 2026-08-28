import { LitElement, html, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import type { TargetSelectionViewModel } from "../application/view-models";
import {
  sharedApplicationStyles,
  sharedIconStyles,
  viewHostStyles,
} from "../styles/component-styles";
import "./lens-target-card";
import {
  dispatchComponentEvent,
  TARGET_SELECTION_INTENT_EVENT,
  type TargetSelectionIntent,
} from "./events";

@customElement("lens-target-selection-view")
export class LensTargetSelectionView extends LitElement {
  static styles = [viewHostStyles, sharedApplicationStyles, ...sharedIconStyles];

  @property({ attribute: false })
  model: TargetSelectionViewModel | undefined;

  protected render() {
    const model = this.model;
    if (!model) return nothing;
    const selection = model.lens.selection;
    const items = selection?.items ?? [];
    const pickerActive = selection?.stage === "picking";
    const operationAvailable = Boolean(model.lens.operation_id && selection);
    const canAdd = Boolean(
      operationAvailable && !pickerActive && items.length < (selection?.maximum_targets ?? 0),
    );
    const canEdit = Boolean(operationAvailable && !pickerActive && !model.pending);

    return html`
      <section
        class="target-selection-shell"
        data-entrance="slide-in-from-right"
        aria-label="Selected windows"
        @lens-target-remove=${this.removeTarget}
      >
        <header class="target-selection-toolbar">
          <output class="target-selection-count" aria-label="Selected window count">
            ${items.length}<span aria-hidden="true"> / ${selection?.maximum_targets ?? 0}</span>
          </output>
          <div class="target-selection-actions" aria-label="Selection actions">
            <button
              type="button"
              class="target-selection-icon-button"
              aria-label="Add another window"
              title="Add another window"
              ?disabled=${!canAdd || model.pending}
              @click=${() => this.emit({ type: "add" })}
            >
              <i class="fa-solid fa-plus" aria-hidden="true"></i>
            </button>
            <button
              type="button"
              class="target-selection-icon-button is-primary"
              aria-label="Use selected windows"
              title="Use selected windows"
              ?disabled=${!canEdit || items.length === 0}
              @click=${() => this.emit({ type: "confirm" })}
            >
              <i class="fa-solid fa-check" aria-hidden="true"></i>
            </button>
          </div>
        </header>

        <ol class="target-selection-list" aria-label="Window previews">
          ${items.map(
            (item) => html`
              <li class="target-selection-card">
                <lens-target-card .item=${item} .disabled=${!canEdit}></lens-target-card>
              </li>
            `,
          )}
        </ol>

        <p class="visually-hidden" role="status" aria-live="polite">
          ${
            model.message ||
            selection?.notice ||
            (pickerActive ? "Choose one window in the system picker." : "")
          }
        </p>
      </section>
    `;
  }

  private removeTarget = (event: CustomEvent<{ targetId: string }>): void => {
    event.stopPropagation();
    this.emit({ type: "remove", targetId: event.detail.targetId });
  };

  private emit(intent: TargetSelectionIntent): void {
    dispatchComponentEvent(this, TARGET_SELECTION_INTENT_EVENT, intent);
  }
}
