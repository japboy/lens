import "./lens-select";
import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { AgentDefaults, AgentSelectionState, ToolPolicies, ToolPolicy } from "../types";
import { AGENT_INTENT_EVENT, dispatchComponentEvent, type AgentIntent } from "./events";
import { sameAgent } from "../view-model";
import { LensSelect, type SelectOption } from "./lens-select";
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
    other: "ask",
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
  { key: "other", label: "Other requests" },
];

@customElement("lens-agent-defaults")
export class LensAgentDefaults extends LitElement {
  @property({ attribute: false }) selection: AgentSelectionState | undefined;
  @property({ attribute: false }) defaults: AgentDefaults | undefined;
  @property({ type: Boolean }) disabled = false;
  @state() private draft: AgentDefaults = structuredClone(DEFAULT_AGENT_DEFAULTS);
  protected createRenderRoot() {
    return this;
  }
  private catalogRefresh:
    | { generation: string; revision: number; stage: "waiting" | "requested" }
    | undefined;
  @state() private resetChoices: string[] = [];

  protected willUpdate(changed: PropertyValues<this>) {
    const previous = changed.get("selection");
    const sameCandidate = sameAgent(previous?.candidate, this.selection?.candidate);
    const runtimeChanged =
      changed.has("selection") &&
      previous &&
      sameCandidate &&
      this.selection?.catalog_generation &&
      previous.catalog_generation !== this.selection.catalog_generation;
    if (runtimeChanged) {
      this.resetChoices = [];
      const model = this.selection?.config_options?.find((option) => option.category === "model");
      // Only the model catalog is authoritative before the draft model is resolved.
      if (model) this.reconcileChoices(new Set([model.id]));
      const draftModel = model ? this.choiceValue(model.id) : "";
      const savedModel =
        this.defaults?.choices.find((choice) => choice.config_id === model?.id)?.value ?? "";
      if (model && draftModel !== savedModel) {
        this.catalogRefresh = {
          generation: this.selection!.catalog_generation!,
          revision: this.selection?.catalog_revision ?? 0,
          stage: "waiting",
        };
      } else {
        this.catalogRefresh = undefined;
        this.reconcileChoices();
      }
    } else if (
      (changed.has("selection") && !sameCandidate) ||
      (changed.has("defaults") &&
        JSON.stringify(changed.get("defaults") ?? DEFAULT_AGENT_DEFAULTS) !==
          JSON.stringify(this.defaults ?? DEFAULT_AGENT_DEFAULTS))
    ) {
      this.draft = this.editableDefaults(this.defaults ?? DEFAULT_AGENT_DEFAULTS);
      this.catalogRefresh = undefined;
      this.resetChoices = [];
    }
    if (
      this.catalogRefresh &&
      this.selection?.catalog_generation === this.catalogRefresh.generation &&
      (this.selection.catalog_revision ?? 0) > this.catalogRefresh.revision &&
      (this.selection.catalog_model ?? "") === this.draftModel()
    ) {
      this.catalogRefresh = undefined;
      this.reconcileChoices();
    }
  }

  protected updated() {
    if (this.catalogRefresh?.stage === "waiting" && !this.disabled) this.refreshModel();
  }

  private reconcileChoices(onlyIds?: Set<string>) {
    const options = this.selection?.config_options;
    const removed: string[] = [];
    this.draft = {
      ...this.draft,
      choices: this.draft.choices.filter((choice) => {
        if (onlyIds && !onlyIds.has(choice.config_id)) return true;
        const option = options?.find((option) => option.id === choice.config_id);
        const valid = options
          ? option?.type === "select" &&
            agentOptionChoices(option).some((item) => item.value === choice.value)
          : choice.config_id === "mode" &&
            this.selection?.modes?.some((mode) => mode.id === choice.value);
        if (!valid) removed.push(option?.name ?? choice.config_id);
        return valid;
      }),
    };
    this.resetChoices = [...new Set([...this.resetChoices, ...removed])];
  }

  private draftModel(): string {
    const model = this.selection?.config_options?.find((option) => option.category === "model");
    return model ? this.choiceValue(model.id) : "";
  }

  private refreshModel() {
    const model = this.selection?.config_options?.find((option) => option.category === "model");
    if (!model || !this.catalogRefresh || this.disabled) return;
    this.catalogRefresh = {
      ...this.catalogRefresh,
      revision: this.selection?.catalog_revision ?? 0,
      stage: "requested",
    };
    dispatchComponentEvent<AgentIntent>(this, AGENT_INTENT_EVENT, {
      type: "preview-model",
      configId: model.id,
      value: this.choiceValue(model.id) || undefined,
    });
    this.requestUpdate();
  }
  private choiceValue(configId: string): string {
    return this.draft.choices.find((choice) => choice.config_id === configId)?.value ?? "";
  }
  private externallyManaged(configId: string): boolean {
    if (typeof this.selection?.candidate !== "object") return false;
    const options = this.selection.config_options;
    if (!options) return configId !== "mode";
    const category = options.find((option) => option.id === configId)?.category;
    return !category || !["model", "mode", "thought_level"].includes(category);
  }
  private editableDefaults(defaults: AgentDefaults): AgentDefaults {
    const result = structuredClone(defaults);
    result.choices = result.choices.filter((choice) => !this.externallyManaged(choice.config_id));
    return result;
  }
  private unlistedChoice(configId: string, values: string[]): SelectOption[] {
    const value = this.choiceValue(configId);
    return value && !values.includes(value)
      ? [{ value, label: `${value} (not in current choices)`, disabled: true }]
      : [];
  }
  protected render() {
    if (this.selection?.stage !== "selected") return nothing;
    const options = this.selection?.config_options;
    const modes = this.selection?.modes ?? [];
    const model = options?.find((o) => o.category === "model");
    const savedModel = this.draft.choices.find((c) => c.config_id === model?.id)?.value;
    const unresolvedModel = savedModel && savedModel !== model?.currentValue;
    return html`<section class="settings-group" aria-labelledby="agent-defaults-heading">
      <h2 id="agent-defaults-heading">Model & Behavior</h2>
      <p class="help">
        Defaults apply to new Lens sessions for this Agent. Saving ends its current session. Choices
        are supplied by the Agent.
      </p>
      ${
        this.catalogRefresh
          ? html`<p role="status">Model settings need to be refreshed for the updated Agent.</p>
              <button ?disabled=${this.disabled} @click=${() => this.refreshModel()}>
                Refresh Model Settings
              </button>`
          : unresolvedModel
            ? html`<p role="status">
                Model settings have not been loaded for this selection. Select the model again to
                refresh its reasoning levels.
              </p>`
            : nothing
      }
      ${this.resetChoices.length ? html`<p role="status">No longer available: ${this.resetChoices.join(", ")}. These choices now use Agent default.</p>` : nothing}
      <fieldset ?disabled=${this.disabled}>
        <legend class="visually-hidden">Session defaults</legend>
        ${
          options
            ? options
                .filter((option) =>
                  ["model", "mode", "thought_level"].includes(option.category ?? ""),
                )
                .map((option) => this.renderOption(option))
            : html`<div class="settings-field">
                <span>Mode default</span>
                <lens-select
                  label="Mode default"
                  data-agent-config-id="mode"
                  .value=${this.choiceValue("mode")}
                  .disabled=${this.disabled}
                  .options=${[
                    { value: "", label: "Agent default" },
                    ...this.unlistedChoice(
                      "mode",
                      modes.map((mode) => mode.id),
                    ),
                    ...modes.map((mode) => ({ value: mode.id, label: mode.name })),
                  ]}
                  @change=${(e: Event) => this.choose("mode", (e.target as LensSelect).value)}
                ></lens-select>
              </div>`
        }
        ${
          !options?.some((o) => o.category === "thought_level")
            ? html`<div class="settings-field">
                <span>Reasoning effort</span
                ><lens-select
                  label="Reasoning effort default"
                  disabled
                  .options=${[{ value: "", label: "Unavailable for the selected model" }]}
                ></lens-select
                ><span class="help">Choose a model to load its supported reasoning levels.</span>
              </div>`
            : nothing
        }
      </fieldset>
      ${
        options?.some(
          (option) => !["model", "mode", "thought_level"].includes(option.category ?? ""),
        )
          ? html`<details class="settings-disclosure">
              <summary>Additional Agent Options</summary>
              <fieldset ?disabled=${this.disabled}>
                ${options.filter((option) => !["model", "mode", "thought_level"].includes(option.category ?? "")).map((option) => this.renderOption(option))}
              </fieldset>
            </details>`
          : nothing
      }
      <details class="settings-disclosure">
        <summary>Tool Approval Policies</summary>
        <fieldset ?disabled=${this.disabled}>
          <legend>Permission request response policy</legend>
          ${EFFECTS.map(
            ({ key, label }) =>
              html`<div class="settings-field">
                <span>${label}</span>
                <lens-select
                  label=${`${label} policy`}
                  .disabled=${this.disabled}
                  .value=${this.draft.tools[key]}
                  .options=${[
                    { value: "ask", label: "Ask each time" },
                    { value: "allow", label: "Automatically approve" },
                    { value: "deny", label: "Automatically reject" },
                  ]}
                  @change=${(e: Event) => this.setPolicy(key, (e.target as LensSelect).value as ToolPolicy)}
                ></lens-select
                >${key === "other" ? html`<span class="help">Requests without a recognized classification, including HTML output publication.</span>` : nothing}
              </div>`,
          )}
        </fieldset>
        <p class="help">
          Applies only to permission requests sent by this Agent, including in future sessions.
          Operations without a request follow the Agent’s own settings and mode. Other requests use
          the policy above; unsupported requests are never automatically approved. Forms and URL
          requests always require a response.
        </p>
      </details>
      <button
        ?disabled=${this.disabled || Boolean(this.catalogRefresh)}
        @click=${() => dispatchComponentEvent<AgentIntent>(this, AGENT_INTENT_EVENT, { type: "save-defaults", defaults: this.editableDefaults(this.draft) })}
      >
        Save Defaults
      </button>
      <button
        ?disabled=${this.disabled}
        @click=${() => {
          this.draft = this.editableDefaults(this.defaults ?? DEFAULT_AGENT_DEFAULTS);
          this.resetChoices = [];
          if (this.catalogRefresh)
            this.catalogRefresh = {
              ...this.catalogRefresh,
              revision: this.selection?.catalog_revision ?? 0,
              stage: "requested",
            };
          const model = this.selection?.config_options?.find((o) => o.category === "model");
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
  private renderOption(option: NonNullable<AgentSelectionState["config_options"]>[number]) {
    if (this.externallyManaged(option.id)) {
      const current = (option.options ?? [])
        .flatMap((choice) => ("group" in choice ? choice.options : [choice]))
        .find((choice) => choice.value === option.currentValue);
      return html`<div class="settings-field">
        <span>${option.name}</span>
        <output aria-label=${`${option.name} managed by external CLI`}
          >${current?.name ?? String(option.currentValue)}</output
        >
        <span class="help"
          >Change this advanced option with the agent's own CLI, then use Save and Verify in
          Connection. Lens uses the agent's configuration.</span
        >
      </div>`;
    }
    return html`<div class="settings-field">
      <span>${option.name}</span>
      <lens-select
        label=${`${option.name} default`}
        data-agent-config-id=${option.id}
        .value=${this.choiceValue(option.id)}
        .disabled=${this.disabled || option.type !== "select" || (Boolean(this.catalogRefresh) && option.category !== "model")}
        .options=${[
          { value: "", label: "Agent default" },
          ...this.unlistedChoice(
            option.id,
            agentOptionChoices(option).map((choice) => choice.value),
          ),
          ...agentOptionChoices(option),
        ]}
        @change=${(e: Event) => this.choose(option.id, (e.target as LensSelect).value)}
      ></lens-select>
      <span class="help"
        >${option.description ?? ""}${option.type !== "select" ? " This control type is not supported." : ""}</span
      >
    </div>`;
  }
  private choose(configId: string, value: string) {
    if (this.externallyManaged(configId)) return;
    const isModel = this.selection?.config_options?.some(
      (o) => o.id === configId && o.category === "model",
    );
    const reasoningIds = new Set(
      this.selection?.config_options
        ?.filter((o) => o.category === "thought_level")
        .map((o) => o.id),
    );
    const choices = this.draft.choices.filter(
      (c) => c.config_id !== configId && !(isModel && reasoningIds.has(c.config_id)),
    );
    if (value) choices.push({ config_id: configId, value });
    this.draft = { ...this.draft, choices };
    if (isModel && this.catalogRefresh)
      this.catalogRefresh = {
        ...this.catalogRefresh,
        revision: this.selection?.catalog_revision ?? 0,
        stage: "requested",
      };
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
