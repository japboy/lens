import { LitElement, html, css, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import { controlStyles, viewHostStyles } from "../styles/component-styles";
export interface SettingsRecoveryInfo {
  message: string;
  settings_path: string;
  can_restore_prompt_presets: boolean;
  digest: string | null;
}
@customElement("lens-settings-recovery-view")
export class LensSettingsRecoveryView extends LitElement {
  static styles = [
    viewHostStyles,
    controlStyles,
    css`
      main {
        padding: 24px;
        max-width: 640px;
        margin: auto;
      }
      .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 12px;
        margin-top: 24px;
      }
      p {
        line-height: 1.5;
        overflow-wrap: anywhere;
      }
    `,
  ];
  @property({ attribute: false }) info: SettingsRecoveryInfo | undefined;
  @property({ type: Boolean }) busy = false;
  @property() error = "";
  @property({ type: Boolean }) confirming = false;
  protected render() {
    return html`<main>
      <h1>Settings need attention</h1>
      ${
        this.info
          ? html`<p role="alert">${this.info.message}</p>
              <p>
                Your settings file has not been replaced. Agent connections remain stopped until the
                settings can be loaded.
              </p>
              <p>${this.info.settings_path}</p>
              ${this.confirming ? html`<p>Restore only Prompt Presets to the bundled defaults? Saved preset edits will be replaced. Other settings will be preserved, and the original file will be backed up. Lens will restart after recovery.</p>` : nothing}
              <div class="actions">
                ${this.confirming ? html`<button type="button" data-lens-button-role="destructive" ?disabled=${this.busy} @click=${() => this.emit("restore")}>Restore Prompt Presets</button><button type="button" data-lens-button-role="cancel" ?disabled=${this.busy} @click=${() => this.emit("cancel")}>Cancel</button>` : html`<button type="button" data-lens-button-role="normal" ?disabled=${this.busy} @click=${() => this.emit("open")}>Open Settings File</button><button type="button" data-lens-button-role="normal" ?disabled=${this.busy} @click=${() => this.emit("retry")}>Retry Loading</button>${this.info.can_restore_prompt_presets && this.info.digest ? html`<button type="button" data-lens-button-role="normal" ?disabled=${this.busy} @click=${() => this.emit("confirm")}>Restore Default Prompt Presets…</button>` : nothing}`}
              </div>`
          : html`<p>Checking settings recovery options…</p>`
      }
      ${this.busy ? html`<p role="status">Working…</p>` : nothing}
      ${this.error ? html`<p role="alert">${this.error}</p>` : nothing}
    </main>`;
  }
  private emit(action: string) {
    this.dispatchEvent(
      new CustomEvent("settings-recovery-action", {
        detail: action,
        bubbles: true,
        composed: true,
      }),
    );
  }
}
