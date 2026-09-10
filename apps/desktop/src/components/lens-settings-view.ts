import { initialSettingsState } from "../rendering/initial-state";
import { renderSnapshotFailure } from "../rendering/snapshot-status";
import { LitElement, css, html, nothing } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { SettingsDestination } from "../agent-prompt-template";
import type {
  SettingsFeedback,
  SettingsFeedbackMessage,
  SettingsViewModel,
} from "../application/view-models";
import {
  accessibilityStyles,
  controlStyles,
  feedbackStyles,
  viewHostStyles,
} from "../styles/component-styles";
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
  static styles = [
    viewHostStyles,
    controlStyles,
    css`
      lens-agent-settings,
      lens-prompt-settings {
        display: contents;
      }

      .settings-shell {
        --settings-sidebar-width: 188px;
        --settings-detail-inline-padding: 26px;
        --settings-content-max-width: 680px;
        width: 100%;
        height: 100dvh;
        min-width: 0;
        min-height: 0;
        display: grid;
        grid-template-columns: var(--settings-sidebar-width) minmax(0, 1fr);
        grid-template-rows: minmax(0, 1fr);
        overflow: hidden;
        background: var(--settings-backdrop);
      }

      .settings-shell > *,
      .settings-group {
        min-width: 0;
      }

      .settings-sidebar {
        grid-column: 1;
        grid-row: 1;
        min-height: 0;
        display: grid;
        grid-template-rows: minmax(0, 1fr) auto auto;
        padding: 18px 10px;
        overflow: hidden;
        border-right: 1px solid var(--settings-group-border);
        background: var(--settings-sidebar-background);
      }

      .settings-sidebar nav {
        min-height: 0;
        display: grid;
        align-content: start;
        gap: 20px;
        overflow-x: hidden;
        overflow-y: auto;
        overscroll-behavior: contain;
      }

      .settings-nav-group {
        display: grid;
        gap: 3px;
      }

      .settings-nav-group-label {
        margin: 0;
        padding: 0 8px 4px;
        color: GrayText;
        font-size: 11px;
        font-weight: 650;
        letter-spacing: 0.01em;
      }

      .settings-nav-item {
        appearance: none;
        width: 100%;
        min-height: 28px;
        padding: 4px 9px;
        overflow: hidden;
        border: 0;
        border-radius: 6px;
        color: CanvasText;
        background: transparent;
        text-align: left;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .settings-nav-item:hover:not(:disabled):not([aria-current="page"]) {
        background: color-mix(in srgb, CanvasText 7%, transparent);
      }

      .settings-nav-item[aria-current="page"] {
        color: var(--settings-selection-foreground);
        background: var(--settings-selection-background);
        font-weight: 600;
      }

      .settings-shell[data-window-emphasis="unemphasized"] .settings-nav-item[aria-current="page"] {
        color: var(--settings-selection-unemphasized-foreground);
        background: var(--settings-selection-unemphasized-background);
      }

      .settings-nav-item:focus-visible {
        outline: 3px solid var(--settings-focus-ring);
        outline-offset: 1px;
      }

      .settings-sidebar-status {
        min-width: 0;
        display: grid;
        gap: 2px;
        margin-top: 12px;
        padding: 10px 8px 0;
        border-top: 1px solid var(--settings-group-border);
        color: GrayText;
        font-size: 11px;
        line-height: 1.35;
      }

      .settings-sidebar-status-label {
        font-weight: 650;
      }

      .settings-sidebar-status-value {
        min-width: 0;
        overflow-wrap: anywhere;
      }

      .settings-detail {
        grid-column: 2;
        grid-row: 1;
        min-width: 0;
        min-height: 0;
        padding: 24px var(--settings-detail-inline-padding);
        overflow-x: hidden;
        overflow-y: auto;
        overscroll-behavior: contain;
      }

      .settings-detail-panel {
        width: 100%;
        min-width: 0;
        min-height: 100%;
      }

      .settings-detail-panel[hidden] {
        display: none;
      }

      .settings-detail-header {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 16px;
        margin-bottom: 20px;
      }

      .settings-detail-header h1 {
        margin: 0;
        font-size: 20px;
        font-weight: 700;
        letter-spacing: -0.015em;
      }

      .settings-detail-header p {
        max-width: 54rem;
        margin: 5px 0 0;
        color: GrayText;
        font-size: 12px;
      }

      .settings-detail-groups {
        display: grid;
        gap: 16px;
        max-width: var(--settings-content-max-width);
      }

      .settings-context-feedback {
        max-width: var(--settings-content-max-width);
        margin: 0;
        padding: 7px 9px;
        border-radius: 6px;
        color: GrayText;
        background: color-mix(in srgb, CanvasText 5%, transparent);
        font-size: 12px;
        line-height: 1.4;
        overflow-wrap: anywhere;
      }

      .settings-detail-panel > .settings-context-feedback {
        margin-bottom: 16px;
      }

      :host([data-platform="macos"]) .settings-group {
        padding: 16px;
        border: 1px solid var(--settings-group-border);
        border-radius: 10px;
        background: var(--settings-group-background);
      }

      fieldset {
        margin: 0;
        padding: 0;
        border: 0;
        display: flex;
        flex-wrap: wrap;
        gap: 10px 24px;
      }

      .radio-row {
        display: inline-flex;
        align-items: center;
        gap: 6px;
      }

      .directory-row,
      .permission-row {
        display: flex;
        align-items: center;
        gap: 8px;
      }

      .directory-row input {
        min-width: 0;
        flex: 1;
      }

      .prompt-editor {
        flex: 1 1 auto;
        width: 100%;
        min-height: 220px;
        font: inherit;
        line-height: 1.45;
        tab-size: 2;
      }

      :host([data-platform="macos"]) .settings-shell :is(.directory-field, .prompt-editor) {
        appearance: none;
        border: 1px solid ButtonBorder;
        border-radius: 5px;
        color: FieldText;
        background: Field;
        box-shadow: inset 0 1px 1px color-mix(in srgb, CanvasText 8%, transparent);
      }

      :host([data-platform="macos"])
        .settings-shell
        :is(.directory-field, .prompt-editor):focus-visible {
        border-color: var(--settings-focus-ring);
        outline: 3px solid var(--settings-focus-ring);
        outline-offset: 1px;
      }

      :host([data-platform="macos"])
        .settings-shell
        :is(.directory-field, .prompt-editor):disabled {
        border-color: color-mix(in srgb, ButtonBorder 65%, transparent);
        color: GrayText;
        background: color-mix(in srgb, Field 72%, Canvas);
        box-shadow: none;
        cursor: default;
      }

      :host([data-platform="macos"]) .settings-shell .directory-field {
        min-height: 24px;
        padding: 3px 7px;
        line-height: 16px;
      }

      :host([data-platform="macos"]) .settings-shell .prompt-editor {
        padding: 6px 8px;
        resize: none;
      }

      .prompt-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 8px;
        margin-top: 12px;
      }

      .prompt-action-spacer {
        flex: 1 1 auto;
      }

      .settings-shell output,
      .settings-shell .help {
        display: block;
        overflow-wrap: anywhere;
      }

      .agent-actions {
        display: flex;
        flex-wrap: wrap;
        justify-content: flex-end;
        gap: 8px;
        margin-top: 10px;
      }

      .status-ok {
        color: LinkText;
      }

      .runtime-status {
        display: grid;
        gap: 6px;
        margin: 8px 0;
      }

      .runtime-status progress {
        width: min(100%, 360px);
      }

      .runtime-message {
        color: GrayText;
      }

      .prompt-workspace {
        position: relative;
        width: 100%;
        min-width: 0;
        min-height: 100%;
        display: grid;
        align-content: start;
        gap: 16px;
        max-width: var(--settings-content-max-width);
      }

      .prompt-detail-header {
        padding-inline-end: 112px;
        margin-bottom: 0;
      }

      .prompt-draft-status {
        position: absolute;
        inset-block-start: 0;
        inset-inline-end: 0;
        flex: 0 0 auto;
        padding: 3px 7px;
        border-radius: 999px;
        color: GrayText;
        background: color-mix(in srgb, CanvasText 7%, transparent);
        font-size: 11px;
        white-space: nowrap;
      }

      .prompt-draft-status[data-dirty="true"] {
        color: MarkText;
        background: Mark;
      }

      .prompt-composition,
      .prompt-editor-section,
      .prompt-preview-section {
        min-width: 0;
        border: 1px solid var(--settings-group-border);
        border-radius: 10px;
        background: var(--settings-group-background);
      }

      .prompt-composition,
      .prompt-editor-section {
        padding: 14px;
      }

      .prompt-composition-heading-row {
        display: flex;
        align-items: flex-end;
        justify-content: space-between;
        flex-wrap: wrap;
        gap: 10px 16px;
        margin-bottom: 12px;
      }

      .prompt-composition-heading-row h2,
      .prompt-editor-header h2 {
        margin: 0;
      }

      .prompt-composition-heading-row p,
      .prompt-editor-header p {
        margin: 4px 0 0;
        color: GrayText;
        font-size: 12px;
        line-height: 1.4;
      }

      .prompt-request-mode {
        display: grid;
        gap: 4px;
        min-width: min(100%, 190px);
        color: CanvasText;
        font-size: 11px;
        font-weight: 600;
      }

      .prompt-request-mode select {
        width: 100%;
        min-width: 0;
      }

      .prompt-composition-flow {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr) auto minmax(0, 1fr);
        align-items: stretch;
        gap: 6px;
      }

      .prompt-composition-part,
      .prompt-composition-result {
        min-width: 0;
        display: grid;
        align-content: start;
        gap: 5px;
        padding: 9px;
        border: 1px solid var(--settings-group-border);
        border-radius: 7px;
        color: CanvasText;
        background: color-mix(in srgb, CanvasText 3%, transparent);
      }

      .prompt-composition-part {
        cursor: default;
      }

      .prompt-composition-part[data-active="true"] {
        border-color: color-mix(in srgb, AccentColor 62%, var(--settings-group-border));
        background: color-mix(in srgb, AccentColor 8%, var(--settings-group-background));
      }

      .prompt-composition-part:has(input:focus-visible) {
        outline: 3px solid var(--settings-focus-ring);
        outline-offset: 1px;
      }

      .prompt-composition-title {
        display: flex;
        align-items: flex-start;
        gap: 5px;
      }

      .prompt-composition-title input {
        flex: 0 0 auto;
        margin: 1px 0 0;
      }

      .prompt-composition-part > span:last-child,
      .prompt-composition-result > span {
        color: GrayText;
        font-size: 10px;
        line-height: 1.35;
      }

      .prompt-composition-operator {
        align-self: center;
        color: GrayText;
        font-size: 15px;
        font-weight: 650;
      }

      .prompt-editor-header {
        margin-bottom: 10px;
      }

      .prompt-editor-header > div > span:first-child {
        display: block;
        margin-bottom: 3px;
        color: GrayText;
        font-size: 10px;
        font-weight: 650;
        letter-spacing: 0.03em;
        text-transform: uppercase;
      }

      .prompt-variable-bar {
        display: flex;
        align-items: center;
        flex-wrap: wrap;
        gap: 6px;
        color: GrayText;
        font-size: 12px;
      }

      .prompt-variable-token {
        appearance: none;
        display: inline-flex;
        align-items: center;
        gap: 5px;
        min-height: 26px;
        padding: 3px 7px;
        border: 1px solid var(--settings-group-border);
        border-radius: 999px;
        color: CanvasText;
        background: var(--settings-group-background);
        font-size: 11px;
      }

      .prompt-variable-token:not(:disabled):hover {
        background: color-mix(in srgb, CanvasText 6%, var(--settings-group-background));
      }

      .prompt-variable-token:focus-visible {
        outline: 3px solid var(--settings-focus-ring);
        outline-offset: 1px;
      }

      .prompt-variable-token[data-state="inserted"] {
        border-color: color-mix(in srgb, AccentColor 48%, var(--settings-group-border));
        background: color-mix(in srgb, AccentColor 7%, var(--settings-group-background));
      }

      .prompt-variable-token[data-state="duplicate"] {
        color: MarkText;
        background: Mark;
      }

      .prompt-variable-token code {
        padding: 1px 4px;
        border-radius: 4px;
        background: color-mix(in srgb, CanvasText 7%, transparent);
        font-family: ui-monospace, "SFMono-Regular", Menlo, monospace;
        font-size: 11px;
      }

      .prompt-variable-state {
        color: GrayText;
        font-size: 10px;
      }

      .prompt-no-variables {
        font-style: italic;
      }

      .prompt-form {
        min-height: 0;
        display: grid;
        gap: 0;
      }

      .prompt-editor-meta {
        flex: 0 0 auto;
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
      }

      .prompt-editor-meta p {
        margin-top: 6px;
      }

      .prompt-character-count {
        flex: 0 0 auto;
        margin-top: 7px;
        color: GrayText;
        font-size: 11px;
        font-variant-numeric: tabular-nums;
      }

      .prompt-character-count[data-over-limit="true"] {
        color: MarkText;
        background: Mark;
      }

      .prompt-validation {
        margin: 6px 0 0;
        font-size: 12px;
      }

      .prompt-preview-section {
        overflow: clip;
      }

      .prompt-preview-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px 12px;
        padding: 11px 14px;
        border-bottom: 1px solid var(--settings-group-border);
      }

      .prompt-preview-heading {
        min-width: 0;
        display: grid;
        gap: 2px;
      }

      .prompt-preview-heading h2 {
        margin: 0;
      }

      .prompt-preview-heading > span,
      .prompt-preview-status {
        color: GrayText;
        font-size: 11px;
      }

      .prompt-preview-content {
        padding: 14px;
      }

      .prompt-preview-output {
        max-height: 420px;
        min-height: 180px;
        margin: 0;
        padding: 12px;
        overflow: auto;
        border: 1px solid var(--settings-group-border);
        border-radius: 6px;
        color: FieldText;
        background: Field;
        font-family: ui-monospace, "SFMono-Regular", Menlo, monospace;
        font-size: 12px;
        line-height: 1.5;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
      }

      .prompt-preview-errors {
        margin: 0 0 10px;
        padding: 8px 10px;
        border-radius: 6px;
        color: MarkText;
        background: Mark;
        font-size: 12px;
      }

      .prompt-preview-errors p,
      .prompt-preview-errors ul {
        margin: 0;
      }

      .prompt-preview-errors ul {
        padding-left: 18px;
      }

      .prompt-transport-note {
        margin-top: 10px;
        padding: 9px 10px;
        border-radius: 6px;
        color: GrayText;
        background: color-mix(in srgb, AccentColor 7%, transparent);
        font-size: 12px;
        line-height: 1.4;
      }

      .settings-empty-state {
        color: GrayText;
      }

      .primary {
        font-weight: 600;
      }

      .status-warning {
        color: MarkText;
        background: Mark;
      }

      @media (max-height: 700px) {
        .settings-sidebar {
          padding-block: 12px;
        }
        .settings-detail {
          padding: 16px 20px;
        }
        :host([data-platform="macos"]) .settings-group {
          padding: 12px;
        }
        .settings-shell h2 {
          margin-bottom: 6px;
        }
        .settings-shell .prompt-editor {
          min-height: 160px;
        }
        .settings-shell .help {
          margin-top: 4px;
          line-height: 1.3;
        }
        .settings-shell .agent-actions,
        .settings-shell .prompt-actions {
          margin-top: 6px;
        }
        .settings-shell .runtime-status {
          margin: 4px 0;
        }
        .settings-detail-header {
          margin-bottom: 12px;
        }
      }

      @media (prefers-contrast: more) {
        :host([data-platform="macos"]) .settings-group {
          border-width: 2px;
          border-color: CanvasText;
        }

        :host([data-platform="macos"]) .settings-shell :is(.directory-field, .prompt-editor) {
          border-width: 2px;
          border-color: CanvasText;
          box-shadow: none;
        }

        :host([data-platform="macos"])
          .settings-shell
          :is(.directory-field, .prompt-editor):focus-visible {
          border-color: AccentColor;
          outline-width: 4px;
        }
      }

      lens-agent-defaults .settings-field {
        display: grid;
        gap: 4px;
        margin-block: 8px;
      }

      lens-agent-defaults select {
        max-width: 100%;
        min-width: 0;
      }

      lens-agent-defaults {
        display: contents;
      }

      .about-entry {
        margin-top: 12px;
        padding: 12px 8px 0;
        border-top: 1px solid var(--settings-group-border);
      }
      .about-entry button {
        width: 100%;
      }
    `,
    feedbackStyles,
    accessibilityStyles,
  ];

  @property() aboutOpenError = "";
  @property({ attribute: false })
  permission: import("../application/accessibility-permission-controller").AccessibilityPermissionState =
    { stage: "inactive" };
  @property({ type: Boolean }) commandPending = false;
  @property({ type: Boolean }) active = initialSettingsState().active;

  @property({ attribute: false })
  model: SettingsViewModel | undefined = initialSettingsState().model;
  @property({ attribute: false }) snapshotStatus = initialSettingsState().snapshotStatus;

  @state()
  private destination: SettingsDestination = initialSettingsState().destination;

  @state()
  private windowEmphasis: "emphasized" | "unemphasized" = initialSettingsState().windowEmphasis;

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
    const permission = model?.permission ?? this.permission;
    const permissionLabel = (() => {
      switch (permission.stage) {
        case "inactive":
        case "checking":
          return "Checking…";
        case "allowed":
          return "Ready";
        case "required":
          return "Permission required";
        case "failed":
          return "Access check failed";
        case "restricted":
          return "Access restricted";
        case "unsupported":
          return "Not supported";
      }
    })();
    const permissionAllowed = permission.stage === "allowed";
    const generalFeedback = model ? feedbackForDestination(model?.feedback, "general") : undefined;
    const promptFeedback = model
      ? feedbackForDestination(model?.feedback, "agent-prompt")
      : undefined;
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
              ${model?.lensStageLabel ?? nothing}
            </span>
          </div>
          <div class="about-entry">${this.aboutButton()}</div>
        </aside>

        <section class="settings-detail">
          ${renderSnapshotFailure(this.snapshotStatus)}
          ${this.aboutOpenError ? html`<p role="alert">${this.aboutOpenError}</p>` : nothing}
          <div class="settings-detail-panel" ?hidden=${this.destination !== "general"}>
            <header class="settings-detail-header">
              <div>
                <h1>General</h1>
                <p>Choose how Lens connects to an Agent and accesses your Mac.</p>
              </div>
            </header>
            ${renderSettingsFeedback(generalFeedback)}

            <div class="settings-detail-groups">
              <section class="settings-group" aria-labelledby="agent-heading">
                <h2 id="agent-heading">AI Agent</h2>
                <div data-region-error="agent"></div>
                <lens-agent-settings
                  .selection=${model?.agentSelection}
                  .runtime=${model?.agentRuntime}
                  .disabled=${!this.active || !model || model.pending}
                ></lens-agent-settings>
              </section>
              <lens-agent-defaults
                .selection=${model?.agentSelection}
                .defaults=${model?.config?.agent_preferences?.[model?.config.agent]}
                .disabled=${!this.active || !model || model.pending}
              ></lens-agent-defaults>

              <section class="settings-group" aria-labelledby="cwd-heading">
                <h2 id="cwd-heading">Working Directory</h2>
                <div class="directory-row">
                  <input
                    type="text"
                    class="directory-field"
                    aria-label="Working Directory"
                    readonly
                    .value=${model?.config?.working_directory ?? ""}
                  />
                  <button
                    @click=${() => this.emit({ type: "choose-directory" })}
                    ?disabled=${!this.active || !model || model.pending || !model?.config}
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
                    permission.stage !== "required"
                      ? nothing
                      : html`<button
                          @click=${() => this.emit({ type: "request-accessibility-permission" })}
                          ?disabled=${!this.active || this.commandPending}
                        >
                          Open System Settings
                        </button>`
                  }
                </div>
              </section>
            </div>
          </div>

          <div class="settings-detail-panel" ?hidden=${this.destination !== "agent-prompt"}>
            <section class="prompt-workspace" aria-labelledby="agent-prompt-heading">
              <header class="settings-detail-header prompt-detail-header">
                <div>
                  <h1 id="agent-prompt-heading">Agent Prompt</h1>
                  <p>
                    Edit every natural-language instruction Lens can send, and inspect the exact
                    composed result.
                  </p>
                </div>
              </header>
              <div data-region-error="prompt"></div>
              <lens-prompt-settings
                .agentPromptTemplate=${model?.config?.agent_prompt_template}
                .synchronization=${model?.promptSynchronization}
                .feedback=${promptFeedback}
                .disabled=${!this.active || !model || model.pending}
              ></lens-prompt-settings>
            </section>
          </div>
        </section>
      </main>
    `;
  }

  private aboutButton() {
    return html`<button
      type="button"
      ?disabled=${!this.active}
      @click=${() => this.emit({ type: "open-about" })}
    >
      About
    </button>`;
  }

  private navigationButton(destination: SettingsDestination, label: string) {
    return html`<button
      type="button"
      class="settings-nav-item"
      ?disabled=${!this.active}
      aria-current=${this.destination === destination ? "page" : nothing}
      @click=${() => {
        this.destination = destination;
      }}
    >
      ${label}
    </button>`;
  }

  activate(): void {
    this.active = true;
    this.updateWindowEmphasis();
  }

  private updateWindowEmphasis = (): void => {
    if (!this.active) return;
    this.windowEmphasis =
      document.visibilityState === "visible" && document.hasFocus() ? "emphasized" : "unemphasized";
  };

  private forwardAgentIntent = (event: CustomEvent<AgentIntent>): void => {
    event.stopPropagation();
    const intent: SettingsIntent = (() => {
      switch (event.detail.type) {
        case "preview-model":
          return {
            type: "preview-agent-model",
            configId: event.detail.configId,
            value: event.detail.value,
          };
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
