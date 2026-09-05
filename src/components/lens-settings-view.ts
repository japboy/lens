import { LitElement, html, nothing } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { SettingsDestination } from "../agent-prompt-template";
import type {
  SettingsFeedback,
  SettingsFeedbackMessage,
  SettingsViewModel,
} from "../application/view-models";
import { sharedApplicationStyles, viewHostStyles } from "../styles/component-styles";
import "./lens-agent-settings";
import "./lens-agent-defaults";
import "./lens-prompt-settings";
import { renderSettingsFeedback } from "./settings-feedback";
import {
  dispatchComponentEvent,
  SETTINGS_INTENT_EVENT,
  type AgentIntent,
  type PromptIntent,
  type SettingsIntent,
} from "./events";

@customElement("lens-settings-view")
export class LensSettingsView extends LitElement {
  static styles = [viewHostStyles, sharedApplicationStyles];

  @property({ attribute: false })
  model: SettingsViewModel | undefined;

  @state()
  private destination: SettingsDestination = "general";

  @state()
  private windowEmphasis: "emphasized" | "unemphasized" = "emphasized";

  connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener("focus", this.updateWindowEmphasis);
    window.addEventListener("blur", this.updateWindowEmphasis);
    document.addEventListener("visibilitychange", this.updateWindowEmphasis);
    this.updateWindowEmphasis();
  }

  disconnectedCallback(): void {
    window.removeEventListener("focus", this.updateWindowEmphasis);
    window.removeEventListener("blur", this.updateWindowEmphasis);
    document.removeEventListener("visibilitychange", this.updateWindowEmphasis);
    super.disconnectedCallback();
  }

  protected render() {
    const model = this.model;
    if (!model) return nothing;
    const permissionLabel = (() => {
      switch (model.permission.stage) {
        case "inactive":
        case "checking":
          return "Checking…";
        case "allowed":
          return "Allowed";
        case "required":
          return "Permission required";
        case "failed":
          return "Permission check failed";
      }
    })();
    const permissionAllowed = model.permission.stage === "allowed";
    const generalFeedback = feedbackForDestination(model.feedback, "general");
    const promptFeedback = feedbackForDestination(model.feedback, "agent-prompt");
    return html`
      <main
        class="settings-shell"
        aria-label="Settings"
        data-window-emphasis=${this.windowEmphasis}
        @lens-agent-intent=${this.forwardAgentIntent}
        @lens-prompt-intent=${this.forwardPromptIntent}
      >
        <aside class="settings-sidebar">
          <nav aria-label="Settings sections">
            <div class="settings-nav-group">
              ${this.navigationButton("general", "General")}
              ${this.navigationButton("agent-prompt", "Agent Prompt")}
            </div>
          </nav>
          <div class="settings-sidebar-status">
            <span id="lens-status-label" class="settings-sidebar-status-label">Lens Status</span>
            <span
              class="settings-sidebar-status-value"
              role="status"
              aria-labelledby="lens-status-label"
            >
              ${model.lensStageLabel}
            </span>
          </div>
        </aside>

        <section class="settings-detail">
          <div class="settings-detail-panel" ?hidden=${this.destination !== "general"}>
            <header class="settings-detail-header">
              <div>
                <h1>General</h1>
                <p>Choose how Lens connects to an Agent and accesses your Mac.</p>
              </div>
            </header>
            ${renderSettingsFeedback(generalFeedback)}

            <div class="settings-detail-groups">
              <lens-agent-settings
                .selection=${model.agentSelection}
                .runtime=${model.agentRuntime}
                .disabled=${model.pending}
              ></lens-agent-settings>
              <lens-agent-defaults
                .selection=${model.agentSelection}
                .defaults=${model.config?.agent_preferences?.[model.config.agent]}
                .disabled=${model.pending}
              ></lens-agent-defaults>

              <section class="settings-group" aria-labelledby="cwd-heading">
                <h2 id="cwd-heading">Working Directory</h2>
                <div class="directory-row">
                  <input
                    type="text"
                    class="directory-field"
                    aria-label="Working Directory"
                    readonly
                    .value=${model.config?.working_directory ?? ""}
                  />
                  <button
                    @click=${() => this.emit({ type: "choose-directory" })}
                    ?disabled=${model.pending || !model.config}
                  >
                    Choose…
                  </button>
                </div>
                <p class="help">
                  The Agent uses this directory as its cwd when resolving project instructions and
                  memory.
                </p>
              </section>

              <section class="settings-group" aria-labelledby="permission-heading">
                <h2 id="permission-heading">Accessibility</h2>
                <div class="permission-row">
                  <output class=${permissionAllowed ? "status-ok" : "status-warning"}>
                    ${permissionLabel}
                  </output>
                  ${
                    permissionAllowed || model.permission.stage === "checking"
                      ? nothing
                      : html`<button
                          @click=${() => this.emit({ type: "request-accessibility-permission" })}
                          ?disabled=${model.pending}
                        >
                          Open System Settings
                        </button>`
                  }
                </div>
              </section>
            </div>
          </div>

          <div class="settings-detail-panel" ?hidden=${this.destination !== "agent-prompt"}>
            <lens-prompt-settings
              .agentPromptTemplate=${model.config?.agent_prompt_template}
              .synchronization=${model.promptSynchronization}
              .feedback=${promptFeedback}
              .disabled=${model.pending}
            ></lens-prompt-settings>
          </div>
        </section>
      </main>
    `;
  }

  private navigationButton(destination: SettingsDestination, label: string) {
    return html`<button
      type="button"
      class="settings-nav-item"
      aria-current=${this.destination === destination ? "page" : nothing}
      @click=${() => {
        this.destination = destination;
      }}
    >
      ${label}
    </button>`;
  }

  private updateWindowEmphasis = (): void => {
    this.windowEmphasis =
      document.visibilityState === "visible" && document.hasFocus() ? "emphasized" : "unemphasized";
  };

  private forwardAgentIntent = (event: CustomEvent<AgentIntent>): void => {
    event.stopPropagation();
    const intent: SettingsIntent = (() => {
      switch (event.detail.type) {
        case "save-defaults":
          return { type: "save-agent-defaults", defaults: event.detail.defaults };
        case "select":
          return { type: "select-agent", agent: event.detail.agent };
        case "authenticate":
          return {
            type: "authenticate-agent-selection",
            methodId: event.detail.methodId,
          };
        case "reauthenticate":
          return { type: "reauthenticate-agent-selection" };
        case "sign-out":
          return { type: "sign-out-agent-selection" };
      }
    })();
    this.emit(intent);
  };

  private forwardPromptIntent = (event: CustomEvent<PromptIntent>): void => {
    event.stopPropagation();
    this.emit(
      event.detail.type === "save"
        ? {
            type: "save-agent-prompt-template",
            agentPromptTemplate: event.detail.agentPromptTemplate,
          }
        : { type: "reset-agent-prompt-template" },
    );
  };

  private emit(intent: SettingsIntent): void {
    dispatchComponentEvent(this, SETTINGS_INTENT_EVENT, intent);
  }
}

function feedbackForDestination(
  feedback: SettingsFeedback,
  destination: SettingsDestination,
): SettingsFeedbackMessage | undefined {
  if (feedback.stage === "none") return undefined;
  return feedback.target === "application" || feedback.target === destination
    ? feedback
    : undefined;
}
