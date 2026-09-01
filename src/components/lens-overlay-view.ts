import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import appIconUrl from "../../src-tauri/icons/icon.png?url";
import type { OverlayViewModel } from "../application/view-models";
import {
  sharedApplicationStyles,
  sharedIconStyles,
  viewHostStyles,
} from "../styles/component-styles";
import {
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
  { id: "translation", label: "Translation" },
  { id: "source", label: "Source" },
  { id: "diagnostics", label: "Diagnostics" },
] as const;

type LensTab = (typeof LENS_TABS)[number]["id"];

@customElement("lens-overlay-view")
export class LensOverlayView extends LitElement {
  static styles = [viewHostStyles, sharedApplicationStyles, ...sharedIconStyles];

  @property({ attribute: false })
  model: OverlayViewModel | undefined;

  @state()
  private activeTab: LensTab = "translation";

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("model")) return;
    const previous = changed.get("model");
    if (previous?.lens.operation_id !== this.model?.lens.operation_id) {
      this.activeTab = "translation";
    }
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
    const targetLabels = targets.map(({ window }) =>
      window.title ? `${window.application_name} — ${window.title}` : window.application_name,
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
    const progressSnackbar = lensProgressSnackbar(lens.stage);

    return html`
      <div class="overlay-shell" @lens-agent-output-intent=${this.forwardOutputIntent}>
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
            <button
              type="button"
              class="close-button"
              data-tauri-drag-region="false"
              aria-label="Close Lens"
              @click=${() => this.emit({ type: "close" })}
            >
              <span class="close-icon" aria-hidden="true"></span>
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

        <main class="overlay-main" data-progress=${progressSnackbar ? "true" : "false"}>
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
                  <button
                    ?disabled=${model.pending}
                    @click=${() => this.emit({ type: "transform" })}
                  >
                    Try Again
                  </button>
                </section>`
              : nothing
          }
          ${
            lens.stage === "ready" || (lens.stage === "failed" && Boolean(lens.input))
              ? html`<div class="overlay-actions">
                  <button
                    class="primary"
                    ?disabled=${model.pending}
                    @click=${() => this.emit({ type: "transform" })}
                  >
                    Transform with Agent
                  </button>
                </div>`
              : nothing
          }
          ${this.renderActivePanel(lens, sourceJson)}
        </main>

        <footer class="overlay-footer">
          <div class="lens-progress-region" role="status" aria-live="polite" aria-atomic="true">
            ${
              progressSnackbar
                ? html`<div class="lens-progress-snackbar">
                    <i class="fa-solid fa-spinner fa-spin" aria-hidden="true"></i>
                    <span class="lens-progress-copy">
                      <strong>${progressSnackbar.title}</strong>
                      <span>${progressSnackbar.detail}</span>
                    </span>
                  </div>`
                : nothing
            }
          </div>
          <div class="overlay-footer-status" title=${STAGE_LABEL[lens.stage]}>
            <span class="overlay-stage-indicator" aria-hidden="true"></span>
            <span class="overlay-stage">${STAGE_LABEL[lens.stage]}</span>
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

  private renderActivePanel(lens: OverlayViewModel["lens"], sourceJson: string) {
    const activeTab = this.activeTab;
    switch (activeTab) {
      case "translation":
        return html`<section
          id="translation-panel"
          class="lens-panel"
          role="tabpanel"
          aria-labelledby="translation-tab"
          tabindex="0"
        >
          <lens-agent-output .lens=${lens}></lens-agent-output>
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
      @click=${() => {
        this.activeTab = tab;
      }}
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
    this.activeTab = nextTab;
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
}
