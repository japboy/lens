import { html, nothing } from "lit";
import type { SettingsFeedbackMessage } from "../application/view-models";

export function renderSettingsFeedback(feedback: SettingsFeedbackMessage | undefined) {
  if (!feedback) return nothing;
  return feedback.stage === "error"
    ? html`<p class="settings-context-feedback error" role="alert">${feedback.message}</p>`
    : html`<p class="settings-context-feedback" role="status">${feedback.message}</p>`;
}
