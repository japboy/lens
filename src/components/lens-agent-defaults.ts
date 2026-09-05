import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { AgentDefaults, AgentSelectionState, ToolPolicies, ToolPolicy } from "../types";
import { AGENT_INTENT_EVENT, dispatchComponentEvent, type AgentIntent } from "./events";
import { agentOptionChoices } from "./agent-option-choices";

export const DEFAULT_AGENT_DEFAULTS: AgentDefaults = {
  choices: [],
  tools: {
    read: "ask",
    search: "ask",
    fetch: "ask",
    edit: "deny",
    delete: "deny",
    move: "deny",
    execute: "deny",
  },
};
const EFFECTS: { key: keyof ToolPolicies; label: string }[] = [
  { key: "read", label: "Read files or data" },
  { key: "search", label: "Search" },
  { key: "fetch", label: "Retrieve external data" },
  { key: "edit", label: "Edit content" },
  { key: "delete", label: "Delete content" },
  { key: "move", label: "Move content" },
  { key: "execute", label: "Execute commands" },
];

@customElement("lens-agent-defaults")
export class LensAgentDefaults extends LitElement {
  @property({ attribute: false }) selection: AgentSelectionState = {
    stage: "unselected",
    auth_methods: [],
  };
  @property({ attribute: false }) defaults: AgentDefaults | undefined;
  @property({ type: Boolean }) disabled = false;
  @state() private draft: AgentDefaults = structuredClone(DEFAULT_AGENT_DEFAULTS);
  protected createRenderRoot() {
    return this;
  }
  protected willUpdate(changed: PropertyValues<this>) {
    if (
      changed.has("defaults") ||
      (changed.has("selection") && changed.get("selection")?.candidate !== this.selection.candidate)
    ) {
      this.draft = structuredClone(this.defaults ?? DEFAULT_AGENT_DEFAULTS);
    }
  }
  protected render() {
    if (this.selection.stage !== "selected") return nothing;
    const options = this.selection.config_options;
    const modes = this.selection.modes ?? [];
    return html`<section class="settings-group" aria-labelledby="agent-defaults-heading">
      <h2 id="agent-defaults-heading">Shared Agent Settings</h2>
      <p class="help">
        Defaults apply to new Lens sessions for this Agent. Saving ends its current session. Choices
        are supplied by the Agent.
      </p>
      <fieldset ?disabled=${this.disabled}>
        <legend>Session defaults</legend>
        ${
          options
            ? options.map(
                (option) => html`<label class="settings-field"
                  ><span>${option.name}</span>
                  <select
                    aria-label=${`${option.name} default`}
                    .value=${this.draft.choices.find((c) => c.config_id === option.id)?.value ?? ""}
                    ?disabled=${option.type !== "select"}
                    @change=${(e: Event) => this.choose(option.id, (e.target as HTMLSelectElement).value)}
                  >
                    <option value="">
                      ${option.category === "mode" ? "Lens safe default" : "Agent default"}
                    </option>
                    ${agentOptionChoices(option)}
                  </select>
                  <span class="help"
                    >${option.description ?? ""}${option.type !== "select" ? " This control type is not supported." : ""}</span
                  ></label
                >`,
              )
            : html`<label
                >Mode default<select
                  aria-label="Mode default"
                  .value=${this.draft.choices.find((c) => c.config_id === "mode")?.value ?? ""}
                  @change=${(e: Event) => this.choose("mode", (e.target as HTMLSelectElement).value)}
                >
                  <option value="">Lens safe default</option>
                  ${modes.map((m) => html`<option value=${m.id}>${m.name}</option>`)}
                </select></label
              >`
        }
      </fieldset>
      <fieldset ?disabled=${this.disabled}>
        <legend>Tool approval policy</legend>
        ${EFFECTS.map(
          ({ key, label }) =>
            html`<label class="settings-field"
              ><span>${label}</span
              ><select
                aria-label=${`${label} policy`}
                .value=${this.draft.tools[key]}
                @change=${(e: Event) => this.setPolicy(key, (e.target as HTMLSelectElement).value as ToolPolicy)}
              >
                <option value="ask">Ask every time</option>
                <option value="deny">Deny</option>
              </select></label
            >`,
        )}
      </fieldset>
      <p class="help">
        Safe mode still prohibits changes and commands. Unknown effects and mode-switch tools are
        denied. Tool approval never changes the session mode.
      </p>
      <button
        ?disabled=${this.disabled}
        @click=${() => dispatchComponentEvent<AgentIntent>(this, AGENT_INTENT_EVENT, { type: "save-defaults", defaults: structuredClone(this.draft) })}
      >
        Save Shared Settings
      </button>
      <button
        ?disabled=${this.disabled}
        @click=${() => {
          this.draft = structuredClone(this.defaults ?? DEFAULT_AGENT_DEFAULTS);
        }}
      >
        Revert
      </button>
    </section>`;
  }
  private choose(configId: string, value: string) {
    const choices = this.draft.choices.filter((c) => c.config_id !== configId);
    if (value) choices.push({ config_id: configId, value });
    this.draft = { ...this.draft, choices };
  }
  private setPolicy(key: keyof ToolPolicies, value: ToolPolicy) {
    this.draft = { ...this.draft, tools: { ...this.draft.tools, [key]: value } };
  }
}
