import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import appIconUrl from "../../src-tauri/icons/icon-macos.svg?url";
import type { OverlayViewModel } from "../application/view-models";
import type { LensRepresentation, LensState } from "../types";
import { composeOutputMedia } from "../output-media";
import {
  sharedApplicationStyles,
  sharedIconStyles,
  viewHostStyles,
} from "../styles/component-styles";
import {
  lensLiveStatus,
  lensOutputPresentation,
  lensProgressSnackbar,
  lensSourceJson,
  STAGE_LABEL,
  supportedAuthMethods,
} from "../view-model";
import "./lens-agent-output";
import "./lens-extraction-diagnostics";
import "./lens-media-gallery";
import {
  dispatchComponentEvent,
  OVERLAY_INTENT_EVENT,
  type AgentOutputIntent,
  type OverlayIntent,
} from "./events";

const LENS_TABS = [
  { id: "interpretation", label: "Interpretation" },
  { id: "source", label: "Source" },
  { id: "diagnostics", label: "Diagnostics" },
] as const;

type LensTab = (typeof LENS_TABS)[number]["id"];

interface InterpretationScrollPosition {
  readonly top: number;
  readonly wasAtBottom: boolean;
  readonly hadMedia: boolean;
}

@customElement("lens-overlay-view")
export class LensOverlayView extends LitElement {
  static styles = [viewHostStyles, sharedApplicationStyles, ...sharedIconStyles];

  @property({ attribute: false })
  model: OverlayViewModel | undefined;

  @state()
  private activeTab: LensTab = "interpretation";

  @state()
  private displayedRepresentation: LensRepresentation | undefined;

  private synchronizedOperationId: string | undefined;
  private hasSynchronizedOperation = false;
  private pendingScrollPosition: InterpretationScrollPosition | undefined;
  private restoreInterpretationFocus = false;

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("model")) return;
    const previous = changed.get("model");
    if (previous?.lens.operation_id !== this.model?.lens.operation_id) {
      this.activeTab = "interpretation";
    }
    if (this.model) this.synchronizeRepresentation(this.model.lens);
  }

  protected render() {
    const model = this.model;
    if (!model) return nothing;
    const lens = model.lens;
    const context = lens.context;
    const targets = lens.target_set?.targets ?? [];
    const sourceJson = lensSourceJson(lens);
    const activeAgent = lens.agent;
    const authenticationMethods = supportedAuthMethods(lens);
    const targetLabels = targets.map(({ facts }) =>
      facts.title ? `${facts.application_name} — ${facts.title}` : facts.application_name,
    );
    const sourceCountLabel =
      targets.length === 0
        ? "No selected windows"
        : `${targets.length} selected ${targets.length === 1 ? "window" : "windows"}`;
    const sourceContext = targetLabels.length
      ? targetLabels.join(" · ")
      : "No source context is available.";
    const sourceContextTitle = targetLabels.length
      ? targetLabels.join("\n")
      : "No source context is available.";
    const canCancel = lens.stage === "connecting" || lens.stage === "transforming";
    const canRetry =
      Boolean(lens.input) && (lens.stage === "authentication_required" || lens.stage === "failed");
    const progressSnackbar = lensProgressSnackbar(lens.stage);
    const liveStatus = lensLiveStatus(lens.live);
    const displayLens = this.lensWithDisplayedRepresentation(lens);
    const hasSettledRepresentation = Boolean(displayLens.representation);
    const initialProgressStatus = progressSnackbar
      ? {
          title: progressSnackbar.title,
          detail: progressSnackbar.detail,
          busy: true,
          prominent: true,
        }
      : undefined;
    const announcedStatus = hasSettledRepresentation
      ? liveStatus
      : (initialProgressStatus ?? liveStatus);
    const showStatusSnackbar = Boolean(announcedStatus?.prominent);
    const persistentStatus = liveStatus;
    const outputMedia = composeOutputMedia(lensOutputPresentation(displayLens));
    const hasMediaCue =
      this.activeTab === "interpretation" &&
      outputMedia.media.length > 0 &&
      outputMedia.narrative.length > 0;

    return html`
      <div
        class="overlay-shell"
        data-progress=${showStatusSnackbar ? "true" : "false"}
        data-media-cue=${hasMediaCue ? "true" : "false"}
        @lens-agent-output-intent=${this.forwardOutputIntent}
      >
        <header class="overlay-header" data-tauri-drag-region="deep">
          <div class="overlay-brand">
            <img class="overlay-app-icon" src=${appIconUrl} alt="" />
            <h1 class="overlay-title visually-hidden">Lens</h1>
          </div>
          <div class="overlay-header-actions">
            ${
              canCancel
                ? html`<button
                    type="button"
                    class="overlay-header-action"
                    data-tauri-drag-region="false"
                    ?disabled=${model.cancelPending}
                    @click=${() => this.emit({ type: "cancel" })}
                  >
                    Cancel
                  </button>`
                : nothing
            }
            ${
              canRetry
                ? html`<button
                    type="button"
                    class="overlay-header-action"
                    data-tauri-drag-region="false"
                    ?disabled=${model.pending}
                    @click=${() => this.emit({ type: "retry" })}
                  >
                    Retry with Agent
                  </button>`
                : nothing
            }
            ${
              lens.live?.lifecycle === "watching"
                ? html`<button
                    type="button"
                    class="overlay-header-action"
                    data-tauri-drag-region="false"
                    @click=${() => this.emit({ type: "pause" })}
                  >
                    Pause Updates
                  </button>`
                : lens.live?.lifecycle === "paused"
                  ? html`<button
                      type="button"
                      class="overlay-header-action"
                      data-tauri-drag-region="false"
                      @click=${() => this.emit({ type: "resume" })}
                    >
                      Resume Updates
                    </button>`
                  : nothing
            }
            <button
              type="button"
              class="close-button"
              data-tauri-drag-region="false"
              aria-label=${lens.operation_id ? "Stop Lens and close" : "Close Lens"}
              title=${lens.operation_id ? "Stop Lens and close" : "Close Lens"}
              @click=${() => this.emit({ type: "close" })}
            >
              <i class="fa-solid fa-xmark" aria-hidden="true"></i>
            </button>
          </div>
        </header>

        <section class="overlay-source-summary" aria-label="Selected source context">
          <span class="overlay-source-icon" aria-hidden="true">
            <i class="fa-solid fa-window-maximize"></i>
          </span>
          <div class="overlay-source-copy">
            <strong class="overlay-source-count">${sourceCountLabel}</strong>
            <span class="overlay-source-targets" title=${sourceContextTitle}>${sourceContext}</span>
          </div>
        </section>

        <nav class="lens-tabs" aria-label="Lens content">
          <div role="tablist" aria-orientation="horizontal">
            ${LENS_TABS.map(({ id, label }) => this.renderTab(id, label))}
          </div>
        </nav>

        <main class="overlay-main">
          ${model.message ? html`<p class="error" role="alert">${model.message}</p>` : nothing}
          ${lens.error ? html`<p class="error" role="alert">${lens.error}</p>` : nothing}
          ${
            activeAgent?.authentication_message
              ? html`<p class="notice" role="status">${activeAgent.authentication_message}</p>`
              : nothing
          }
          ${
            lens.stage === "authentication_required"
              ? html`<section class="overlay-actions" aria-label="Agent authentication">
                  ${
                    authenticationMethods.length
                      ? authenticationMethods.map(
                          (method) => html`<button
                            ?disabled=${model.pending}
                            @click=${() => this.emit({ type: "authenticate", methodId: method.id })}
                          >
                            Authenticate with ${method.name}…
                          </button>`,
                        )
                      : html`<p>Authenticate with this agent's existing CLI, then try again.</p>`
                  }
                </section>`
              : nothing
          }
          ${this.renderActivePanel(lens, displayLens, sourceJson)}
        </main>

        <div class="lens-progress-region">
          ${
            announcedStatus && showStatusSnackbar
              ? html`<div class="lens-progress-snackbar">
                  <div
                    class="lens-status-announcement"
                    role="status"
                    aria-live="polite"
                    aria-atomic="true"
                  >
                    <i
                      class=${
                        announcedStatus.busy
                          ? "fa-solid fa-spinner fa-spin"
                          : "fa-solid fa-circle-info lens-status-icon"
                      }
                      aria-hidden="true"
                    ></i>
                    <span class="lens-progress-copy">
                      <strong>${announcedStatus.title}</strong>
                      <span>${announcedStatus.detail}</span>
                    </span>
                  </div>
                </div>`
              : announcedStatus
                ? html`<span
                    class="visually-hidden"
                    role="status"
                    aria-live="polite"
                    aria-atomic="true"
                    >${announcedStatus.title}. ${announcedStatus.detail}</span
                  >`
                : nothing
          }
        </div>

        <footer class="overlay-footer">
          <div
            class="overlay-footer-status"
            title=${persistentStatus?.detail ?? STAGE_LABEL[lens.stage]}
          >
            <span class="overlay-stage-indicator" aria-hidden="true"></span>
            <span class="overlay-stage">${persistentStatus?.title ?? STAGE_LABEL[lens.stage]}</span>
          </div>
          ${
            context
              ? html`<span class="quality quality-${context.quality}">${context.quality}</span>`
              : nothing
          }
        </footer>
      </div>
    `;
  }

  private renderActivePanel(
    lens: OverlayViewModel["lens"],
    displayLens: LensState,
    sourceJson: string,
  ) {
    const activeTab = this.activeTab;
    switch (activeTab) {
      case "interpretation":
        return html`<section
          id="interpretation-panel"
          class="lens-panel"
          role="tabpanel"
          aria-labelledby="interpretation-tab"
          tabindex="0"
        >
          <lens-agent-output .lens=${displayLens}></lens-agent-output>
        </section>`;
      case "source":
        return html`<section
          id="source-panel"
          class="lens-panel"
          role="tabpanel"
          aria-labelledby="source-tab"
          tabindex="0"
        >
          ${
            sourceJson
              ? html`<div class="lens-content source-view">
                  <lens-media-gallery .lens=${lens}></lens-media-gallery>
                  <section class="source-json" aria-labelledby="source-json-heading">
                    <h2 id="source-json-heading">Structured input</h2>
                    <pre
                      class="source-content"
                      aria-label="Normalized Lens source JSON"
                    ><code>${sourceJson}</code></pre>
                  </section>
                </div>`
              : html`<div class="lens-content">
                  <p class="empty-state">No normalized source data is available.</p>
                </div>`
          }
        </section>`;
      case "diagnostics":
        return html`<section
          id="diagnostics-panel"
          class="lens-panel"
          role="tabpanel"
          aria-labelledby="diagnostics-tab"
          tabindex="0"
        >
          ${
            lens.context
              ? html`<lens-extraction-diagnostics
                  .context=${lens.context}
                  .agent=${lens.agent}
                ></lens-extraction-diagnostics>`
              : html`<div class="lens-content">
                  <p class="empty-state">No extraction diagnostics are available.</p>
                </div>`
          }
        </section>`;
    }
  }

  private renderTab(tab: LensTab, label: string) {
    const selected = this.activeTab === tab;
    return html`<button
      type="button"
      id="${tab}-tab"
      class="lens-tab"
      role="tab"
      aria-selected=${selected ? "true" : "false"}
      aria-controls="${tab}-panel"
      tabindex=${selected ? 0 : -1}
      @click=${() => this.activateTab(tab)}
      @keydown=${this.handleTabKeyDown}
    >
      ${label}
    </button>`;
  }

  private handleTabKeyDown = (event: KeyboardEvent): void => {
    const activeIndex = LENS_TABS.findIndex(({ id }) => id === this.activeTab);
    const nextIndex = (() => {
      switch (event.key) {
        case "ArrowLeft":
          return (activeIndex - 1 + LENS_TABS.length) % LENS_TABS.length;
        case "ArrowRight":
          return (activeIndex + 1) % LENS_TABS.length;
        case "Home":
          return 0;
        case "End":
          return LENS_TABS.length - 1;
        default:
          return undefined;
      }
    })();
    if (nextIndex === undefined) return;
    const nextTab = LENS_TABS[nextIndex]?.id;
    if (!nextTab) return;
    event.preventDefault();
    this.activateTab(nextTab);
    void this.updateComplete.then(() => {
      this.renderRoot.querySelector<HTMLButtonElement>(`#${nextTab}-tab`)?.focus();
    });
  };

  private forwardOutputIntent = (event: CustomEvent<AgentOutputIntent>): void => {
    event.stopPropagation();
    this.emit(event.detail);
  };

  private emit(intent: OverlayIntent): void {
    dispatchComponentEvent(this, OVERLAY_INTENT_EVENT, intent);
  }

  private activateTab(tab: LensTab): void {
    this.activeTab = tab;
  }

  private synchronizeRepresentation(lens: LensState): void {
    const operationId = lens.operation_id;
    const representation = lens.representation;
    if (!this.hasSynchronizedOperation || operationId !== this.synchronizedOperationId) {
      this.hasSynchronizedOperation = true;
      this.synchronizedOperationId = operationId;
      this.displayedRepresentation = representation;
      return;
    }
    if (!representation) return;
    if (representation.representation_id === this.displayedRepresentation?.representation_id) {
      return;
    }
    this.acceptRepresentation(representation, this.interpretationHasFocus());
  }

  private lensWithDisplayedRepresentation(lens: LensState): LensState {
    const representation = this.displayedRepresentation;
    if (!representation || representation === lens.representation) return lens;
    return { ...lens, representation };
  }

  private interpretationHasFocus(): boolean {
    const panel = this.renderRoot.querySelector<HTMLElement>("#interpretation-panel");
    if (!panel) return false;
    const activeElement = this.shadowRoot?.activeElement;
    return Boolean(activeElement && panel.contains(activeElement));
  }

  private acceptRepresentation(
    representation: LensRepresentation,
    restoreInterpretationFocus = false,
  ): void {
    if (representation.representation_id === this.displayedRepresentation?.representation_id) {
      return;
    }
    this.pendingScrollPosition = this.captureInterpretationScrollPosition();
    this.restoreInterpretationFocus ||= restoreInterpretationFocus;
    this.displayedRepresentation = representation;
    void this.updateComplete.then(() => this.restoreInterpretationPresentation());
  }

  private captureInterpretationScrollPosition(): InterpretationScrollPosition | undefined {
    const output = this.renderRoot
      .querySelector("lens-agent-output")
      ?.querySelector<HTMLElement>(".lens-output");
    if (!output) return undefined;
    const maximum = Math.max(0, output.scrollHeight - output.clientHeight);
    return {
      top: output.scrollTop,
      wasAtBottom: maximum - output.scrollTop <= 36,
      hadMedia: output.classList.contains("has-media"),
    };
  }

  private async restoreInterpretationPresentation(): Promise<void> {
    const outputComponent = this.renderRoot.querySelector<
      HTMLElement & {
        updateComplete: Promise<boolean>;
      }
    >("lens-agent-output");
    await outputComponent?.updateComplete;
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const output = outputComponent?.querySelector<HTMLElement>(".lens-output");
    const position = this.pendingScrollPosition;
    this.pendingScrollPosition = undefined;
    if (output && position) {
      const maximum = Math.max(0, output.scrollHeight - output.clientHeight);
      output.scrollTop = output.classList.contains("has-media")
        ? position.hadMedia
          ? Math.min(position.top, maximum)
          : 0
        : position.wasAtBottom
          ? maximum
          : Math.min(position.top, maximum);
    }
    if (this.restoreInterpretationFocus) {
      this.restoreInterpretationFocus = false;
      this.renderRoot
        .querySelector<HTMLElement>("#interpretation-panel")
        ?.focus({ preventScroll: true });
    }
  }
}
