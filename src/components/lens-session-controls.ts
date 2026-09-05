import { LitElement, html, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import type {
  AgentSessionControlState,
  InteractionResponse,
  ElicitationSchema,
  ElicitationField,
} from "../types";
import { OVERLAY_INTENT_EVENT, dispatchComponentEvent, type OverlayIntent } from "./events";
import { agentOptionChoices } from "./agent-option-choices";

@customElement("lens-session-controls")
export class LensSessionControls extends LitElement {
  @property({ attribute: false }) controls: AgentSessionControlState | undefined;
  protected createRenderRoot() {
    return this;
  }
  protected render() {
    const controls = this.controls;
    if (!controls) return nothing;
    const pending = controls.interactions.filter((i) => i.status === "pending");
    const disabled =
      !controls.active || controls.change?.status === "pending" || pending.length > 0;
    return html`<section aria-label="Agent session controls" class="session-controls">
      <p>
        ${controls.agent_name} · Mode:
        <strong>${controls.effective_mode}</strong>${!controls.active ? " · Session ended" : ""}
      </p>
      ${controls.notice ? html`<p role="status">${controls.notice}</p>` : nothing}
      <details>
        <summary>Session settings</summary>
        <p class="help">
          Changes here apply only to this session. Shared defaults are available in Settings.
        </p>
        ${
          controls.config_options
            ? controls.config_options.map(
                (option) =>
                  html`<label
                    ><span>${option.name}</span
                    ><select
                      aria-label=${option.name}
                      .value=${String(option.currentValue)}
                      ?disabled=${disabled || option.type !== "select"}
                      @change=${(event: Event) => this.choose(option.id, (event.target as HTMLSelectElement).value)}
                    >
                      ${agentOptionChoices(option)}</select
                    ><span class="help">${option.description ?? ""}</span></label
                  >`,
              )
            : html`<label
                >Mode<select
                  aria-label="Mode"
                  .value=${controls.effective_mode}
                  ?disabled=${disabled}
                  @change=${(event: Event) => this.choose("mode", (event.target as HTMLSelectElement).value)}
                >
                  ${controls.modes.map((mode) => html`<option value=${mode.id}>${mode.name}</option>`)}
                </select></label
              >`
        }
        ${controls.change ? html`<p role="status">Settings change: ${controls.change.status}</p>` : nothing}
      </details>
      ${pending.map((interaction) => {
        const details = interaction.details;
        if (!details) return nothing;
        return html`<section role="group" aria-label="Agent decision required">
          ${
            details.kind === "mode_transition"
              ? html`<h3>Change session mode?</h3>
                  <p>${details.from} → ${details.to}</p>
                  <p>
                    This can increase the Agent's authority for this session. Tool approval remains
                    separate.
                  </p>
                  <button
                    ?disabled=${!controls.active}
                    @click=${() => this.respond(interaction.id, { action: "accept" })}
                  >
                    Confirm mode change</button
                  ><button @click=${() => this.respond(interaction.id, { action: "decline" })}>
                    Keep current mode
                  </button>`
              : details.kind === "form"
                ? this.form(interaction.id, details.message, details.schema)
                : details.kind === "url"
                  ? html`<h3>Open the requested URL?</h3>
                      <p>${details.message}</p>
                      <p class="elicitation-url">${details.url}</p>
                      <button @click=${() => this.respond(interaction.id, { action: "accept" })}>
                        Open URL and continue</button
                      ><button @click=${() => this.respond(interaction.id, { action: "decline" })}>
                        Decline
                      </button>`
                  : html`<h3>${details.title}</h3>
                      <p>Requested effect: ${details.effect}</p>
                      <p>Tool call: ${details.tool_call_id}</p>
                      <pre>${JSON.stringify(details.arguments, null, 2)}</pre>
                      ${details.options.map((option) => html`<button ?disabled=${!controls.active} @click=${() => this.respond(interaction.id, { action: "select", option_id: option.optionId })}>${option.name}</button>`)}`
          }
          <button @click=${() => this.respond(interaction.id, { action: "cancel" })}>
            Cancel request
          </button>
        </section>`;
      })}
      ${controls.interactions.length && !pending.length ? html`<p role="status">Last interaction: ${controls.interactions.at(-1)?.status}</p>` : nothing}
    </section>`;
  }
  private form(id: string, message: string, schema: ElicitationSchema) {
    return html`<form @submit=${(event: SubmitEvent) => this.submitForm(event, id, schema)}>
      <h3>${schema.title ?? "Agent input requested"}</h3>
      <p>${message}</p>
      <p>${schema.description ?? ""}</p>
      ${Object.entries(schema.properties).map(([name, field]) => html`<label class="session-form-field"><span>${field.title ?? name}${schema.required?.includes(name) ? " (required)" : ""}</span>${this.formField(name, field, schema.required?.includes(name) ?? false)}<span class="help">${field.description ?? ""}</span></label>`)}
      <button type="submit">Send response</button
      ><button type="button" @click=${() => this.respond(id, { action: "decline" })}>
        Decline
      </button>
    </form>`;
  }
  private formField(name: string, field: ElicitationField, required: boolean) {
    const values = field.type === "array" ? field.items : field;
    const choices = values?.oneOf ?? values?.enum?.map((value) => ({ const: value, title: value }));
    if (choices)
      return html`<select name=${name} ?multiple=${field.type === "array"} ?required=${required}>
        <option value="">Choose…</option>
        ${choices.map((c) => html`<option value=${c.const}>${c.title}</option>`)}
      </select>`;
    if (field.type === "boolean")
      return html`<select name=${name} ?required=${required}>
        <option value="">Choose…</option>
        <option value="true">Yes</option>
        <option value="false">No</option>
      </select>`;
    if (field.type === "number" || field.type === "integer")
      return html`<input
        name=${name}
        type="number"
        step=${field.type === "integer" ? "1" : "any"}
        min=${field.minimum ?? nothing}
        max=${field.maximum ?? nothing}
        ?required=${required}
      />`;
    return html`<input
      name=${name}
      type="text"
      minlength=${field.minLength ?? nothing}
      maxlength=${field.maxLength ?? 16384}
      ?required=${required}
      autocomplete="off"
    />`;
  }
  private submitForm(event: SubmitEvent, id: string, schema: ElicitationSchema) {
    event.preventDefault();
    const form = event.currentTarget as HTMLFormElement;
    if (!form.reportValidity()) return;
    const values = new FormData(form);
    const content: Record<string, string | number | boolean | string[]> = {};
    for (const [name, field] of Object.entries(schema.properties)) {
      const value = values.get(name);
      if (field.type === "array") {
        const selected = values
          .getAll(name)
          .filter((v): v is string => typeof v === "string" && v !== "");
        if (selected.length || schema.required?.includes(name)) content[name] = selected;
      } else if (typeof value === "string" && value !== "") {
        content[name] =
          field.type === "boolean"
            ? value === "true"
            : field.type === "number" || field.type === "integer"
              ? Number(value)
              : value;
      }
    }
    this.respond(id, { action: "submit", content });
  }
  private choose(configId: string, value: string) {
    const controls = this.controls;
    if (controls)
      this.emit({
        type: "set-session-option",
        instanceId: controls.instance_id,
        revision: controls.config_revision,
        configId,
        value,
      });
  }
  private respond(interactionId: string, response: InteractionResponse) {
    if (this.controls)
      this.emit({
        type: "respond-interaction",
        instanceId: this.controls.instance_id,
        interactionId,
        response,
      });
  }
  private emit(intent: OverlayIntent) {
    dispatchComponentEvent(this, OVERLAY_INTENT_EVENT, intent);
  }
}
