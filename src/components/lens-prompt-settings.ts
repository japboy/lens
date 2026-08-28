import { LitElement, html, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { dispatchComponentEvent, PROMPT_INTENT_EVENT, type PromptIntent } from "./events";

@customElement("lens-prompt-settings")
export class LensPromptSettings extends LitElement {
  @property({ attribute: false })
  responsePrompt: string | undefined;

  @property({ type: Boolean })
  disabled = false;

  @state()
  private draft = "";

  @state()
  private dirty = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("responsePrompt")) return;
    if (!this.dirty || this.responsePrompt === this.draft) {
      this.draft = this.responsePrompt ?? "";
      this.dirty = false;
    }
  }

  protected render() {
    return html`
      <section class="settings-group" aria-labelledby="prompt-heading">
        <h2 id="prompt-heading">Agent Prompt</h2>
        <form @submit=${this.save}>
          <textarea
            class="prompt-editor"
            aria-label="Agent Prompt"
            required
            .value=${this.draft}
            @input=${this.edit}
            ?disabled=${this.disabled || this.responsePrompt === undefined}
          ></textarea>
          <p class="help">
            Controls how the Agent transforms the source. Lens appends fixed source-data boundaries
            and safety instructions when it sends the prompt.
          </p>
          <div class="prompt-actions">
            <button
              type="button"
              @click=${() => this.emit({ type: "reset" })}
              ?disabled=${this.disabled || this.responsePrompt === undefined}
            >
              Reset to Default…
            </button>
            <button
              type="submit"
              class="primary"
              ?disabled=${this.disabled || !this.dirty || !this.draft.trim()}
            >
              Save Prompt
            </button>
          </div>
        </form>
      </section>
    `;
  }

  private edit = (event: Event): void => {
    this.draft = (event.currentTarget as HTMLTextAreaElement).value;
    this.dirty = this.draft !== this.responsePrompt;
  };

  private save = (event: SubmitEvent): void => {
    event.preventDefault();
    if (!this.dirty || !this.draft.trim()) return;
    this.emit({ type: "save", responsePrompt: this.draft });
  };

  private emit(intent: PromptIntent): void {
    dispatchComponentEvent(this, PROMPT_INTENT_EVENT, intent);
  }
}
