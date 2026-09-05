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
      (changed.has("defaults") &&
        JSON.stringify(changed.get("defaults") ?? DEFAULT_AGENT_DEFAULTS) !==
          JSON.stringify(this.defaults ?? DEFAULT_AGENT_DEFAULTS)) ||
      (changed.has("selection") && changed.get("selection")?.candidate !== this.selection.candidate)
    ) {
      this.draft = structuredClone(this.defaults ?? DEFAULT_AGENT_DEFAULTS);
    }
  }
  protected render() {
    if (this.selection.stage !== "selected") return nothing;
    const options = this.selection.config_options;
    const modes = this.selection.modes ?? [];
    const model = options?.find((o) => o.category === "model");
    const savedModel = this.draft.choices.find((c) => c.config_id === model?.id)?.value;
    const unresolvedModel = savedModel && savedModel !== model?.currentValue;
    return html`<section class="settings-group" aria-labelledby="agent-defaults-heading">
      <h2 id="agent-defaults-heading">Shared Agent Settings</h2>
      <p class="help">
        Defaults apply to new Lens sessions for this Agent. Saving ends its current session. Choices
        are supplied by the Agent.
      </p>
      ${unresolvedModel ? html`<p role="status">Model settings have not been loaded for this selection. Select the model again to refresh its reasoning levels.</p>` : nothing}
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
        ${
          !options?.some((o) => o.category === "thought_level")
            ? html`<label class="settings-field"
                ><span>Reasoning effort</span
                ><select aria-label="Reasoning effort default" disabled>
                  <option>Unavailable for the selected model</option></select
                ><span class="help"
                  >Choose a model to load its supported reasoning levels.</span
                ></label
              >`
            : nothing
        }
      </fieldset>
      <fieldset ?disabled=${this.disabled}>
        <legend>Permission request response policy</legend>
        ${EFFECTS.map(
          ({ key, label }) =>
            html`<label class="settings-field"
              ><span>${label}</span
              ><select
                aria-label=${`${label} policy`}
                .value=${this.draft.tools[key]}
                @change=${(e: Event) => this.setPolicy(key, (e.target as HTMLSelectElement).value as ToolPolicy)}
              >
                <option value="ask">Ask each time</option>
                <option value="allow">Automatically approve</option>
                <option value="deny">Automatically reject</option>
              </select></label
            >`,
        )}
      </fieldset>
      <p class="help">
        Applies only to permission requests sent by this Agent, including in future sessions.
        Operations without a request follow the Agent’s own settings and mode. Unclassified requests
        require confirmation; unsupported requests are never automatically approved. Forms and URL
        requests always require a response.
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
          const model = this.selection.config_options?.find((o) => o.category === "model");
          if (model)
            dispatchComponentEvent<AgentIntent>(this, AGENT_INTENT_EVENT, {
              type: "preview-model",
              configId: model.id,
              value: this.draft.choices.find((c) => c.config_id === model.id)?.value,
            });
        }}
      >
        Revert
      </button>
    </section>`;
  }
  private choose(configId: string, value: string) {
    const isModel = this.selection.config_options?.some(
      (o) => o.id === configId && o.category === "model",
    );
    const reasoningIds = new Set(
      this.selection.config_options?.filter((o) => o.category === "thought_level").map((o) => o.id),
    );
    const choices = this.draft.choices.filter(
      (c) => c.config_id !== configId && !(isModel && reasoningIds.has(c.config_id)),
    );
    if (value) choices.push({ config_id: configId, value });
    this.draft = { ...this.draft, choices };
    if (isModel)
      dispatchComponentEvent<AgentIntent>(this, AGENT_INTENT_EVENT, {
        type: "preview-model",
        configId,
        value: value || undefined,
      });
  }
  private setPolicy(key: keyof ToolPolicies, value: ToolPolicy) {
    this.draft = { ...this.draft, tools: { ...this.draft.tools, [key]: value } };
  }
}
