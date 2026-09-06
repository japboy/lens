import { initialTargetSelectionState } from "../rendering/initial-state";
import { renderSnapshotFailure } from "../rendering/snapshot-status";
import { LitElement, html, nothing, type PropertyValues } from "lit";
import { repeat } from "lit/directives/repeat.js";
import { customElement, property, state } from "lit/decorators.js";
import type { TargetSelectionViewModel } from "../application/view-models";
import type { LensTargetSelectionItem } from "../types";
import {
  sharedApplicationStyles,
  sharedIconStyles,
  viewHostStyles,
} from "../styles/component-styles";
import {
  dispatchComponentEvent,
  TARGET_SELECTION_INTENT_EVENT,
  type TargetSelectionIntent,
} from "./events";

type TargetCardMotion =
  | { stage: "settled" }
  | { stage: "adding"; targetId: string }
  | {
      stage: "removing";
      targetId: string;
      item: LensTargetSelectionItem;
      index: number;
      animationFinished: boolean;
    };

const REMOVE_MOTION_FALLBACK_MS = 500;

@customElement("lens-target-selection-view")
export class LensTargetSelectionView extends LitElement {
  static styles = [viewHostStyles, sharedApplicationStyles, ...sharedIconStyles];

  @property({ attribute: false })
  model: TargetSelectionViewModel | undefined = initialTargetSelectionState().model;
  @property({ attribute: false }) snapshotStatus = initialTargetSelectionState().snapshotStatus;

  @state()
  private cardMotion: TargetCardMotion = initialTargetSelectionState().cardMotion;

  private removeMotionFallback: number | undefined;

  override disconnectedCallback(): void {
    this.clearRemoveMotionFallback();
    super.disconnectedCallback();
  }

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("model")) return;
    const previous = changed.get("model");
    const previousItems = previous?.lens.selection?.items ?? [];
    const items = this.model?.lens.selection?.items ?? [];
    const motion = this.cardMotion;

    if (motion.stage === "removing") {
      const targetRemains = items.some((item) => item.id === motion.targetId);
      const commandFailed = Boolean(previous?.pending) && !this.model?.pending && targetRemains;
      if ((!targetRemains && motion.animationFinished) || commandFailed) {
        this.clearRemoveMotionFallback();
        this.cardMotion = { stage: "settled" };
      }
      return;
    }

    if (motion.stage === "adding") {
      if (!items.some((item) => item.id === motion.targetId)) {
        this.cardMotion = { stage: "settled" };
      }
      return;
    }

    if (previousItems.length === 0 || items.length !== previousItems.length + 1) return;
    const previousIds = new Set(previousItems.map((item) => item.id));
    const added = items.filter((item) => !previousIds.has(item.id));
    if (added.length === 1) {
      this.cardMotion = { stage: "adding", targetId: added[0]!.id };
    }
  }

  protected render() {
    const model = this.model;
    const selection = model?.lens.selection;
    const items = selection?.items ?? [];
    const renderedItems = this.itemsIncludingRemovingCard(items);
    const pickerActive = selection?.stage === "picking";
    const operationAvailable = Boolean(model?.lens.operation_id && selection);
    const motionSettled = this.cardMotion.stage === "settled";
    const canAdd = Boolean(
      operationAvailable &&
      !pickerActive &&
      motionSettled &&
      items.length < (selection?.maximum_targets ?? 0),
    );
    const canEdit = Boolean(
      operationAvailable && !pickerActive && motionSettled && !model?.pending,
    );

    return html`
      <section
        class="target-selection-shell"
        aria-label="Selected windows"
        @lens-target-remove=${this.removeTarget}
      >
        <header class="target-selection-toolbar">
          <output class="target-selection-count" aria-label="Selected window count">
            ${selection ? items.length : "…"}<span aria-hidden="true"
              >${selection ? ` / ${selection.maximum_targets}` : nothing}</span
            >
          </output>
          <div class="target-selection-actions" aria-label="Selection actions">
            <button
              type="button"
              class="target-selection-icon-button"
              aria-label="Add another window"
              title="Add another window"
              ?disabled=${!canAdd || model?.pending}
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

        ${renderSnapshotFailure(this.snapshotStatus)}
        <div data-region-error="preview"></div>
        <ol class="target-selection-list" aria-label="Window previews">
          ${repeat(
            renderedItems,
            (item) => item.id,
            (item) => html`
              <li
                class="target-selection-card"
                data-target-id=${item.id}
                data-motion=${this.cardMotionFor(item.id)}
                @animationend=${(event: AnimationEvent) => this.finishCardMotion(event, item.id)}
              >
                <lens-target-card .item=${item} .disabled=${!canEdit}></lens-target-card>
              </li>
            `,
          )}
        </ol>

        <p class="visually-hidden" role="status" aria-live="polite">
          ${
            model?.message ||
            selection?.notice ||
            (pickerActive ? "Choose one window in the system picker." : nothing)
          }
        </p>
      </section>
    `;
  }

  private removeTarget = (event: CustomEvent<{ targetId: string }>): void => {
    event.stopPropagation();
    if (this.cardMotion.stage !== "settled") return;
    const targetId = event.detail.targetId;
    const items = this.model?.lens.selection?.items ?? [];
    const index = items.findIndex((item) => item.id === targetId);
    if (index < 0) return;
    this.cardMotion = {
      stage: "removing",
      targetId,
      item: items[index]!,
      index,
      animationFinished: false,
    };
    this.clearRemoveMotionFallback();
    this.removeMotionFallback = window.setTimeout(
      () => this.finishRemoveMotion(targetId),
      REMOVE_MOTION_FALLBACK_MS,
    );
    this.emit({ type: "remove", targetId });
  };

  private itemsIncludingRemovingCard(
    items: readonly LensTargetSelectionItem[],
  ): readonly LensTargetSelectionItem[] {
    const motion = this.cardMotion;
    if (motion.stage !== "removing" || items.some((item) => item.id === motion.targetId)) {
      return items;
    }
    const renderedItems = [...items];
    renderedItems.splice(Math.min(motion.index, renderedItems.length), 0, motion.item);
    return renderedItems;
  }

  private cardMotionFor(targetId: string): TargetCardMotion["stage"] {
    return this.cardMotion.stage !== "settled" && this.cardMotion.targetId === targetId
      ? this.cardMotion.stage
      : "settled";
  }

  private finishCardMotion(event: AnimationEvent, targetId: string): void {
    if (event.target !== event.currentTarget) return;
    if (this.cardMotion.stage === "adding" && this.cardMotion.targetId === targetId) {
      this.cardMotion = { stage: "settled" };
      return;
    }
    if (this.cardMotion.stage === "removing" && this.cardMotion.targetId === targetId) {
      this.finishRemoveMotion(targetId);
    }
  }

  private finishRemoveMotion(targetId: string): void {
    if (this.cardMotion.stage !== "removing" || this.cardMotion.targetId !== targetId) return;
    this.clearRemoveMotionFallback();
    const targetRemains = Boolean(
      this.model?.lens.selection?.items.some((item) => item.id === targetId),
    );
    this.cardMotion = targetRemains
      ? { ...this.cardMotion, animationFinished: true }
      : { stage: "settled" };
  }

  private clearRemoveMotionFallback(): void {
    if (this.removeMotionFallback === undefined) return;
    window.clearTimeout(this.removeMotionFallback);
    this.removeMotionFallback = undefined;
  }

  private emit(intent: TargetSelectionIntent): void {
    dispatchComponentEvent(this, TARGET_SELECTION_INTENT_EVENT, intent);
  }
}
