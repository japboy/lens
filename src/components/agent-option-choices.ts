import { html } from "lit";
import type { SessionConfigOption } from "../types";

export function agentOptionChoices(option: SessionConfigOption) {
  return (option.options ?? []).map((choice) =>
    "group" in choice
      ? html`<optgroup label=${choice.name}>
          ${choice.options.map((value) => html`<option value=${value.value} title=${value.description ?? ""}>${value.name}</option>`)}
        </optgroup>`
      : html`<option value=${choice.value} title=${choice.description ?? ""}>
          ${choice.name}
        </option>`,
  );
}
