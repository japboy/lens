import { initialTargetSelectionState } from "../rendering/initial-state";
import { renderSnapshotFailure } from "../rendering/snapshot-status";
import { LitElement, css, html, nothing, type PropertyValues } from "lit";
import { repeat } from "lit/directives/repeat.js";
import { customElement, property, state } from "lit/decorators.js";
import type { TargetSelectionViewModel } from "../application/view-models";
import type { LensTargetSelectionItem } from "../types";
import {
  accessibilityStyles,
  controlStyles,
  reducedMotionStyles,
  viewHostStyles,
} from "../styles/component-styles";
import { sharedIconStyles } from "../styles/icon-styles";
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
  static styles = [
    viewHostStyles,
    controlStyles,
    css`
      lens-target-card {
        display: contents;
      }

      .target-selection-shell {
        width: 100%;
        height: 100dvh;
        display: flex;
        flex-direction: column;
        overflow: hidden;
        border: 1px solid color-mix(in srgb, Separator 78%, transparent);
        border-radius: 14px;
        background: color-mix(in srgb, Canvas 86%, transparent);
        color: CanvasText;
        box-shadow: 0 14px 38px color-mix(in srgb, CanvasText 24%, transparent);
      }

      .target-selection-toolbar {
        flex: 0 0 48px;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 10px;
        padding: 9px 10px 7px 14px;
      }

      .target-selection-count {
        color: GrayText;
        font-size: 12px;
        font-variant-numeric: tabular-nums;
      }

      .target-selection-count span {
        opacity: 0.72;
      }

      .target-selection-actions {
        display: flex;
        align-items: center;
        gap: 5px;
      }

      .target-selection-icon-button,
      .target-selection-remove {
        appearance: none;
        display: grid;
        place-items: center;
        border: 0;
        padding: 0;
        color: GrayText;
        cursor: default;
      }

      .target-selection-icon-button {
        width: 28px;
        min-width: 28px;
        height: 28px;
        min-height: 28px;
        border-radius: 50%;
        background: color-mix(in srgb, CanvasText 7%, transparent);
        font-size: 11px;
      }

      .target-selection-icon-button.is-primary {
        color: AccentColorText;
        background: AccentColor;
      }

      .target-selection-icon-button:is(:hover, :focus-visible):not(:disabled) {
        color: CanvasText;
        background: color-mix(in srgb, CanvasText 14%, transparent);
      }

      .target-selection-icon-button.is-primary:is(:hover, :focus-visible):not(:disabled) {
        color: AccentColorText;
        background: color-mix(in srgb, AccentColor 86%, CanvasText 14%);
      }

      .target-selection-icon-button:focus-visible,
      .target-selection-remove:focus-visible {
        outline: 3px solid color-mix(in srgb, AccentColor 48%, transparent);
        outline-offset: 1px;
      }

      .target-selection-icon-button:disabled,
      .target-selection-remove:disabled {
        opacity: 0.38;
      }

      .target-selection-list {
        flex: 1 1 auto;
        min-height: 0;
        margin: 0;
        padding: 0 10px 10px;
        display: flex;
        flex-direction: column;
        gap: 10px;
        overflow-x: hidden;
        overflow-y: auto;
        overscroll-behavior: contain;
        list-style: none;
      }

      .target-selection-card {
        flex: 0 0 140px;
        min-width: 0;
        overflow: hidden;
        border: 1px solid color-mix(in srgb, Separator 80%, transparent);
        border-radius: 11px;
        background: color-mix(in srgb, Canvas 76%, CanvasText 2%);
      }

      .target-selection-card[data-motion="adding"] {
        animation: target-selection-card-add 180ms ease-out both;
      }

      .target-selection-card[data-motion="removing"] {
        pointer-events: none;
        animation: target-selection-card-remove 180ms ease-in both;
      }

      @keyframes target-selection-card-add {
        from {
          opacity: 0;
          transform: translateX(28px);
        }

        to {
          opacity: 1;
          transform: translateX(0);
        }
      }

      @keyframes target-selection-card-remove {
        from {
          opacity: 1;
          transform: translateX(0);
        }

        to {
          opacity: 0;
          transform: translateX(28px);
        }
      }

      .target-selection-image {
        position: relative;
        height: 102px;
        overflow: hidden;
        border-bottom: 1px solid color-mix(in srgb, Separator 68%, transparent);
        background: color-mix(in srgb, CanvasText 7%, Canvas);
      }

      .target-selection-image > img,
      .target-selection-placeholder {
        position: absolute;
        inset: 0;
        width: 100%;
        height: 100%;
      }

      .target-selection-image > img {
        object-fit: cover;
        object-position: top center;
      }

      .target-selection-placeholder {
        display: grid;
        place-items: center;
        color: color-mix(in srgb, GrayText 56%, transparent);
        font-size: 24px;
      }

      .target-selection-remove {
        position: absolute;
        z-index: 1;
        top: 7px;
        right: 7px;
        width: 22px;
        min-width: 22px;
        height: 22px;
        min-height: 22px;
        border-radius: 50%;
        color: white;
        background: color-mix(in srgb, black 64%, transparent);
        box-shadow: 0 1px 4px color-mix(in srgb, black 32%, transparent);
        font-size: 10px;
      }

      .target-selection-remove:is(:hover, :focus-visible):not(:disabled) {
        background: color-mix(in srgb, black 82%, transparent);
      }

      .target-selection-caption {
        min-width: 0;
        height: 37px;
        padding: 5px 9px 6px;
        display: flex;
        flex-direction: column;
        justify-content: center;
        gap: 1px;
        line-height: 1.15;
      }

      .target-selection-caption strong,
      .target-selection-caption span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .target-selection-caption strong {
        font-size: 11px;
        font-weight: 650;
      }

      .target-selection-caption span {
        color: GrayText;
        font-size: 10px;
      }
    `,
    accessibilityStyles,
    reducedMotionStyles,
    ...sharedIconStyles,
  ];

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
