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
        border: var(--floating-window-border, 1px solid ButtonBorder);
        border-radius: var(--floating-window-corner-radius, 0px);
        background: var(--floating-window-background, Canvas);
        color: CanvasText;
        box-shadow: var(
          --floating-window-shadow,
          0 14px 38px color-mix(in srgb, CanvasText 24%, transparent)
        );
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
        display: grid;
        place-items: center;
        padding: 0;
      }

      .target-selection-icon-button {
        width: 28px;
        min-width: 28px;
        height: 24px;
        min-height: 24px;
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

      .target-selection-image-action {
        position: absolute;
        z-index: 1;
        top: 7px;
        right: 7px;
        display: grid;
        width: 24px;
        height: 24px;
        border-radius: var(--button-radius, 5px);
        background: var(--window-background, Canvas);
      }

      .target-selection-remove {
        width: 24px;
        min-width: 24px;
        height: 24px;
        min-height: 24px;
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
  private defaultFocusConsidered = false;

  override connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener("keydown", this.handleDefaultKey);
  }

  override disconnectedCallback(): void {
    this.removeEventListener("keydown", this.handleDefaultKey);
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

  protected updated(): void {
    if (this.defaultFocusConsidered || !this.canConfirm) return;
    this.defaultFocusConsidered = true;
    const focused = this.shadowRoot?.activeElement;
    if (!focused && (document.activeElement === document.body || document.activeElement === this)) {
      this.shadowRoot
        ?.querySelector<HTMLFormElement>("form.target-selection-shell")
        ?.focus({ preventScroll: true });
    }
  }

  private get canConfirm(): boolean {
    const selection = this.model?.lens.selection;
    return Boolean(
      this.model?.lens.operation_id &&
      selection &&
      selection.stage !== "picking" &&
      this.cardMotion.stage === "settled" &&
      !this.model?.pending &&
      selection.items.length > 0,
    );
  }

  private confirmSelection = (event: SubmitEvent): void => {
    event.preventDefault();
    if (this.canConfirm) this.emit({ type: "confirm" });
  };

  private handleDefaultKey = (event: KeyboardEvent): void => {
    if (
      event.key !== "Enter" ||
      event.defaultPrevented ||
      event.isComposing ||
      event.repeat ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey ||
      !this.canConfirm
    )
      return;
    const form = this.shadowRoot?.querySelector<HTMLFormElement>("form.target-selection-shell");
    const focused = this.shadowRoot?.activeElement;
    if (!form || (focused && focused !== form)) return;
    const interactive =
      'button, input, textarea, select, lens-select, a[href], [contenteditable]:not([contenteditable="false"]), [role="button"], [role="combobox"], [tabindex]';
    if (
      event
        .composedPath()
        .some(
          (node) =>
            node instanceof Element && node !== this && node !== form && node.matches(interactive),
        )
    )
      return;
    const submitter = form.querySelector<HTMLButtonElement>('button[type="submit"]');
    if (!submitter || submitter.disabled) return;
    event.preventDefault();
    form.requestSubmit(submitter);
  };

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
      <form
        class="target-selection-shell"
        tabindex="-1"
        @submit=${this.confirmSelection}
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
              data-lens-button-role="normal"
              type="button"
              class="target-selection-icon-button"
              aria-label="Add Another Window"
              title="Add Another Window"
              ?disabled=${!canAdd || model?.pending}
              @click=${() => this.emit({ type: "add" })}
            >
              <i class="fa-solid fa-plus" aria-hidden="true"></i>
            </button>
            <button
              type="submit"
              data-lens-button-role="primary"
              class="target-selection-icon-button"
              aria-label="Use Selected Windows"
              title="Use Selected Windows"
              ?disabled=${!this.canConfirm}
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
      </form>
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
