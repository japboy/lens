import { LitElement, html, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import type { SettingsViewModel } from "../application/view-models";
import { sharedApplicationStyles, viewHostStyles } from "../styles/component-styles";
import "./lens-agent-settings";
import "./lens-prompt-settings";
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

    return html`
      <main
        class="settings-shell"
        aria-label="Settings"
        @lens-agent-intent=${this.forwardAgentIntent}
        @lens-prompt-intent=${this.forwardPromptIntent}
      >
        <lens-agent-settings
          .selection=${model.agentSelection}
          .runtime=${model.agentRuntime}
          .disabled=${model.pending}
        ></lens-agent-settings>

        <lens-prompt-settings
          .responsePrompt=${model.config?.response_prompt}
          .disabled=${model.pending}
        ></lens-prompt-settings>

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
            The default is your home directory. The agent uses this directory as the cwd for
            resolving its own project instructions and memory.
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

        <footer>
          <span role="status">${model.message || model.lensStageLabel}</span>
        </footer>
      </main>
    `;
  }

  private forwardAgentIntent = (event: CustomEvent<AgentIntent>): void => {
    event.stopPropagation();
    const intent: SettingsIntent = (() => {
      switch (event.detail.type) {
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
        ? { type: "save-response-prompt", responsePrompt: event.detail.responsePrompt }
        : { type: "reset-response-prompt" },
    );
  };

  private emit(intent: SettingsIntent): void {
    dispatchComponentEvent(this, SETTINGS_INTENT_EVENT, intent);
  }
}
