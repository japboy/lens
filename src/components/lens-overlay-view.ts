import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { OverlayViewModel } from "../application/view-models";
import {
  sharedApplicationStyles,
  sharedIconStyles,
  viewHostStyles,
} from "../styles/component-styles";
import { lensSourceJson, STAGE_LABEL, supportedAuthMethods } from "../view-model";
import "./lens-agent-output";
import "./lens-extraction-diagnostics";
import "./lens-media-gallery";
import {
  dispatchComponentEvent,
  OVERLAY_INTENT_EVENT,
  type AgentOutputIntent,
  type OverlayIntent,
} from "./events";

type LensTab = "translation" | "source";

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
    const firstWindow = targets[0]?.window;
    const applicationName =
      targets.length > 1 ? `${targets.length} Windows` : (firstWindow?.application_name ?? "Lens");
    const windowContext = targets.length
      ? targets
          .map(({ window }) =>
            window.title ? `${window.application_name} — ${window.title}` : window.application_name,
          )
          .join("\n")
      : applicationName;

    return html`
      <div class="overlay-shell" @lens-agent-output-intent=${this.forwardOutputIntent}>
        <header class="overlay-header" data-tauri-drag-region="deep">
          <h1 class="overlay-title" title=${windowContext}>
            <strong>${applicationName}</strong>
            ${
              targets.length === 1 && firstWindow?.title
                ? html`<span> — ${firstWindow.title}</span>`
                : targets.length > 1
                  ? html`<span> — Combined context</span>`
                  : nothing
            }
          </h1>
          <button
            type="button"
            class="close-button"
            data-tauri-drag-region="false"
            aria-label="Close Lens"
            @click=${() => this.emit({ type: "close" })}
          >
            <span class="close-icon" aria-hidden="true"></span>
          </button>
        </header>

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
          ${
            lens.stage === "connecting" || lens.stage === "transforming"
              ? html`<div class="overlay-actions">
                  <button
                    ?disabled=${model.cancelPending}
                    @click=${() => this.emit({ type: "cancel" })}
                  >
                    Cancel
                  </button>
                </div>`
              : nothing
          }
          ${
            this.activeTab === "translation"
              ? html`<section
                  id="translation-panel"
                  class="lens-panel"
                  role="tabpanel"
                  aria-labelledby="translation-tab"
                  tabindex="0"
                >
                  <lens-agent-output .lens=${lens}></lens-agent-output>
                </section>`
              : html`<section
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
                      : html`<p class="empty-state">No normalized source data is available.</p>`
                  }
                  <lens-extraction-diagnostics
                    .context=${context}
                    .agent=${lens.agent}
                  ></lens-extraction-diagnostics>
                </section>`
          }
        </main>

        <footer class="overlay-footer">
          <div class="overlay-footer-meta">
            <div class="overlay-footer-status" role="status" title=${STAGE_LABEL[lens.stage]}>
              <span class="overlay-stage">${STAGE_LABEL[lens.stage]}</span>
              ${
                context
                  ? html`<span class="quality quality-${context.quality}">${context.quality}</span>`
                  : nothing
              }
            </div>
          </div>
          <div class="lens-tabs" role="tablist" aria-label="Lens content">
            ${this.renderTab("translation", "Translation")} ${this.renderTab("source", "Source")}
          </div>
        </footer>
      </div>
    `;
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
    const nextTab = (() => {
      switch (event.key) {
        case "ArrowLeft":
        case "ArrowRight":
          return this.activeTab === "translation" ? "source" : "translation";
        case "Home":
          return "translation";
        case "End":
          return "source";
        default:
          return undefined;
      }
    })();
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
