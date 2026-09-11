import { LitElement, html, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import type { AgentKind, AgentRuntimeState, AgentSelectionState } from "../types";
import {
  AGENT_RUNTIME_LABEL,
  AGENT_SELECTION_LABEL,
  isAgentRuntimeActive,
  selectedAgent,
} from "../view-model";
import { AGENT_INTENT_EVENT, dispatchComponentEvent, type AgentIntent } from "./events";

@customElement("lens-agent-settings")
export class LensAgentSettings extends LitElement {
  @property({ attribute: false })
  selection: AgentSelectionState | undefined;

  @property({ attribute: false })
  runtime: AgentRuntimeState | undefined;

  @property({ type: Boolean })
  disabled = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    if (!this.selection || !this.runtime) return nothing;
    const controlsDisabled =
      this.disabled ||
      isAgentRuntimeActive(this.runtime.stage) ||
      ["checking", "authenticating", "signing_out"].includes(this.selection.stage);
    const methods =
      this.selection.stage === "authentication_required"
        ? this.selection.auth_methods.filter((method) => method.supported)
        : [];
    const selected = selectedAgent(this.selection);
    const selectedLabel = selected === "claude" ? "Claude" : "Codex";
    // An unknown total arrives as `null`, so normalise before the indeterminate check:
    // `=== undefined` would never match and lit would render `max=""`, which HTML treats
    // as max=1 — a determinate bar pinned by however many bytes have arrived.
    const total = this.runtime.total_bytes ?? undefined;

    return html`
      <fieldset ?disabled=${controlsDisabled}>
        <legend class="visually-hidden">AI agent to use</legend>
        ${this.agentOption("claude", "Claude")} ${this.agentOption("codex", "Codex")}
      </fieldset>
      <p class="help">The ACP agent, not Lens, manages authentication credentials.</p>
      <div class="runtime-status" aria-live="polite">
        <output class=${this.runtime.stage === "failed" ? "status-warning" : "runtime-message"}>
          ${this.runtime.error ?? this.runtime.message ?? AGENT_RUNTIME_LABEL[this.runtime.stage]}
        </output>
        ${
          this.runtime.stage === "downloading"
            ? total === undefined
              ? html`<progress aria-label="Agent runtime download progress"></progress>`
              : html`<progress
                  aria-label="Agent runtime download progress"
                  .value=${this.runtime.downloaded_bytes}
                  max=${total}
                ></progress>`
            : nothing
        }
      </div>
      <output class=${this.selection.stage === "selected" ? "status-ok" : "status-warning"}>
        ${
          this.selection.error ??
          this.selection.message ??
          AGENT_SELECTION_LABEL[this.selection.stage]
        }
      </output>
      ${
        this.selection.stage === "authentication_required"
          ? html`<div class="agent-actions" aria-label="Agent authentication">
              ${
                methods.length
                  ? methods.map(
                      (method) => html`
                        <button
                          ?disabled=${this.disabled}
                          @click=${() => this.emit({ type: "authenticate", methodId: method.id })}
                        >
                          Authenticate with ${method.name}…
                        </button>
                      `,
                    )
                  : html`<p>Authenticate with this Agent's existing CLI, then select it again.</p>`
              }
            </div>`
          : nothing
      }
      ${
        selected
          ? html`<div class="agent-actions" aria-label="${selectedLabel} authentication management">
              <button
                ?disabled=${this.disabled}
                @click=${() => this.emit({ type: "reauthenticate" })}
              >
                Reauthenticate…
              </button>
              <button ?disabled=${this.disabled} @click=${() => this.emit({ type: "sign-out" })}>
                Sign Out…
              </button>
            </div>`
          : nothing
      }
    `;
  }

  private agentOption(value: AgentKind, label: string) {
    return html`
      <label class="radio-row">
        <input
          type="radio"
          name="agent"
          value=${value}
          .checked=${this.selection ? selectedAgent(this.selection) === value : false}
          @change=${() => this.emit({ type: "select", agent: value })}
        />
        <span>${label}</span>
      </label>
    `;
  }

  private emit(intent: AgentIntent): void {
    dispatchComponentEvent(this, AGENT_INTENT_EVENT, intent);
  }
}
