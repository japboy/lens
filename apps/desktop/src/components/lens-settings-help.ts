import { LitElement, html } from "lit";
import { customElement, property, state } from "lit/decorators.js";

/** Supplementary Settings information belongs to its own focusable trigger. */
@customElement("lens-settings-help")
export class LensSettingsHelp extends LitElement {
  @property() helpId = "";
  @property() label = "More information";
  @property() text = "";
  @state() private visible = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    return html`<button
        type="button"
        class="settings-info"
        aria-label=${this.label}
        aria-describedby=${this.helpId}
        @pointerenter=${() => {
          this.visible = true;
        }}
        @pointerleave=${() => {
          this.visible = false;
        }}
        @focus=${() => {
          this.visible = true;
        }}
        @blur=${() => {
          this.visible = false;
        }}
        @keydown=${(event: KeyboardEvent) => {
          if (event.key === "Escape") this.visible = false;
        }}
      >
        <i class="fa-solid fa-circle-info" aria-hidden="true"></i></button
      ><span id=${this.helpId} role="tooltip" ?hidden=${!this.visible}>${this.text}</span>`;
  }
}
