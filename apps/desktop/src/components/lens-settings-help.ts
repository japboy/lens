import { LitElement, html } from "lit";
import { customElement, property, state } from "lit/decorators.js";

/** Supplementary Settings information belongs to its own focusable trigger. */
@customElement("lens-settings-help")
export class LensSettingsHelp extends LitElement {
  @property() helpId = "";
  @property() label = "More information";
  @property() text = "";
  @state() private hovered = false;
  @state() private focused = false;
  @state() private dismissed = false;
  private listeningDocument?: Document;

  private get visible(): boolean {
    return !this.dismissed && (this.hovered || this.focused);
  }

  private readonly handlePointerEnter = () => {
    this.hovered = true;
    this.dismissed = false;
  };

  private readonly handlePointerLeave = () => {
    this.hovered = false;
  };

  private readonly handleDocumentKeydown = (event: KeyboardEvent) => {
    if (event.key === "Escape" && this.visible) this.dismissed = true;
  };

  connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener("pointerenter", this.handlePointerEnter);
    this.addEventListener("pointerleave", this.handlePointerLeave);
    this.listeningDocument = this.ownerDocument;
    this.listeningDocument.addEventListener("keydown", this.handleDocumentKeydown);
  }

  disconnectedCallback(): void {
    this.removeEventListener("pointerenter", this.handlePointerEnter);
    this.removeEventListener("pointerleave", this.handlePointerLeave);
    this.listeningDocument?.removeEventListener("keydown", this.handleDocumentKeydown);
    this.listeningDocument = undefined;
    this.hovered = false;
    this.focused = false;
    this.dismissed = false;
    super.disconnectedCallback();
  }

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    return html`<button
        type="button"
        class="settings-info"
        aria-label=${this.label}
        aria-describedby=${this.helpId}
        @focus=${() => {
          this.focused = true;
          this.dismissed = false;
        }}
        @blur=${() => {
          this.focused = false;
        }}
      >
        <i class="fa-solid fa-circle-info" aria-hidden="true"></i></button
      ><span id=${this.helpId} role="tooltip" ?hidden=${!this.visible}>${this.text}</span>`;
  }
}
