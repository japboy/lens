import { keyed } from "lit/directives/keyed.js";
import type { InteractionSubmission } from "../application/view-models";
import { LitElement, html, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import type {
  AgentSessionControlState,
  InteractionResponse,
  ElicitationSchema,
  ElicitationField,
} from "../types";
import { OVERLAY_INTENT_EVENT, dispatchComponentEvent, type OverlayIntent } from "./events";

@customElement("lens-session-controls")
export class LensSessionControls extends LitElement {
  @property({ attribute: false }) controls: AgentSessionControlState | undefined;
  @property() presentation: "diagnostics" | "interaction" = "diagnostics";
  @property({ attribute: false }) submission: InteractionSubmission | undefined;
  protected createRenderRoot() {
    return this;
  }
  protected render() {
    const controls = this.controls;
    if (!controls) return nothing;
    const pending = controls.active
      ? controls.interactions
          .filter((i) => i.status === "pending")
          .sort((a, b) => a.sequence - b.sequence)
      : [];
    if (this.presentation === "diagnostics") {
      return html`<section aria-label="Agent session controls" class="session-controls">
        <p>
          ${controls.agent_name} · Mode:
          <strong>${controls.effective_mode}</strong>${!controls.active ? " · Session ended" : ""}
        </p>
        ${controls.notice ? html`<p role="status">${controls.notice}</p>` : nothing}
        ${controls.interactions.map((i) => html`<p>Interaction ${i.sequence}: ${i.status}</p>`)}
      </section>`;
    }
    const interaction = pending[0];
    if (!interaction?.details) return nothing;
    const details = interaction.details;
    const submission =
      this.submission?.instanceId === controls.instance_id &&
      this.submission.interactionId === interaction.id
        ? this.submission
        : undefined;
    const busy = submission?.stage === "sending" || submission?.stage === "sent";
    return keyed(
      `${controls.instance_id}:${interaction.id}`,
      html` <section
        class="agent-interaction"
        aria-label="Agent decision required"
        aria-busy=${busy}
      >
        <fieldset ?disabled=${busy}>
          ${
            details.kind === "form"
              ? this.form(interaction.id, details.message, details.schema)
              : html` <div class="interaction-body">
                    <h3>
                      ${details.kind === "mode_transition" ? "Change session mode?" : details.kind === "url" ? "Open the requested URL?" : details.title}
                    </h3>
                    ${
                      details.kind === "mode_transition"
                        ? html`<p>${details.from} → ${details.to}</p>
                            <p>
                              This can increase the Agent's authority for this session. Tool
                              approval remains separate.
                            </p>`
                        : details.kind === "url"
                          ? html`<p>${details.message}</p>
                              <p class="elicitation-url">${details.url}</p>`
                          : html`<p>Requested effect: ${details.effect}</p>
                              <pre>${JSON.stringify(details.arguments, null, 2)}</pre>
                              <details>
                                <summary>Request details</summary>
                                <p>Tool call: ${details.tool_call_id}</p>
                              </details>`
                    }
                  </div>
                  <div class="interaction-actions">
                    ${
                      details.kind === "mode_transition"
                        ? html` <button
                              @click=${() => this.respond(interaction.id, { action: "decline" })}
                            >
                              Keep current mode
                            </button>
                            <button
                              @click=${() => this.respond(interaction.id, { action: "accept" })}
                            >
                              Confirm mode change
                            </button>`
                        : details.kind === "url"
                          ? html` <button
                                @click=${() => this.respond(interaction.id, { action: "decline" })}
                              >
                                Decline
                              </button>
                              <button
                                @click=${() => this.respond(interaction.id, { action: "accept" })}
                              >
                                Open URL and continue
                              </button>`
                          : [...details.options]
                              .sort(
                                (a, b) =>
                                  Number(a.kind === "allow_once") - Number(b.kind === "allow_once"),
                              )
                              .map(
                                (option) =>
                                  html`<button
                                    @click=${() => this.respond(interaction.id, { action: "select", option_id: option.optionId })}
                                  >
                                    ${option.name}
                                  </button>`,
                              )
                    }
                    ${details.kind === "permission" && !details.options.some((o) => o.kind === "reject_once") ? html`<button @click=${() => this.respond(interaction.id, { action: "cancel" })}>Cancel request</button>` : nothing}
                  </div>`
          }
        </fieldset>
        ${busy ? html`<p class="interaction-feedback" role="status">Sending response…</p>` : nothing}
        ${submission?.stage === "failed" ? html`<p class="interaction-feedback" role="alert">${submission.message}</p>` : nothing}
        ${pending.length > 1 ? html`<p class="interaction-feedback">${pending.length - 1} more requests waiting</p>` : nothing}
      </section>`,
    );
  }
  private form(id: string, message: string, schema: ElicitationSchema) {
    return html`<form @submit=${(event: SubmitEvent) => this.submitForm(event, id, schema)}>
      <div class="interaction-body">
        <h3>${schema.title ?? "Agent input requested"}</h3>
        <p>${message}</p>
        <p>${schema.description ?? ""}</p>
        ${Object.entries(schema.properties).map(([name, field]) => html`<label class="session-form-field"><span>${field.title ?? name}${schema.required?.includes(name) ? " (required)" : ""}</span>${this.formField(name, field, schema.required?.includes(name) ?? false)}<span class="help">${field.description ?? ""}</span></label>`)}
      </div>
      <div class="interaction-actions">
        <button type="button" @click=${() => this.respond(id, { action: "decline" })}>
          Decline
        </button>
        <button type="submit">Send response</button>
      </div>
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
  private respond(interactionId: string, response: InteractionResponse) {
    const controls = this.controls;
    if (
      !controls?.active ||
      !controls.interactions.some((i) => i.id === interactionId && i.status === "pending")
    )
      return;
    if (
      this.submission?.instanceId === controls.instance_id &&
      this.submission.interactionId === interactionId &&
      this.submission.stage !== "failed"
    )
      return;
    this.emit({
      type: "respond-interaction",
      instanceId: controls.instance_id,
      interactionId,
      response,
    });
  }
  private emit(intent: OverlayIntent) {
    dispatchComponentEvent(this, OVERLAY_INTENT_EVENT, intent);
  }
}
