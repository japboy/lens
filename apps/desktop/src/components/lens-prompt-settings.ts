import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import {
  MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS,
  REQUEST_PROMPT_SECTIONS,
  agentPromptTemplatesEqual,
  cloneAgentPromptTemplate,
  promptRequestSectionDescriptor,
  promptSectionDescriptor,
  renderAgentPromptTemplate,
  templatePlaceholderOccurrences,
  validateAgentPromptTemplate,
  type AgentPromptPreviewMode,
  type PromptEditorLayer,
  type PromptSectionDescriptor,
  type PromptTemplateSection,
  type PromptVariableDescriptor,
} from "../agent-prompt-template";
import type { PromptSynchronization, SettingsFeedbackMessage } from "../application/view-models";
import type { AgentPromptTemplate } from "../types";
import { dispatchComponentEvent, PROMPT_INTENT_EVENT, type PromptIntent } from "./events";
import { renderSettingsFeedback } from "./settings-feedback";

@customElement("lens-prompt-settings")
export class LensPromptSettings extends LitElement {
  @property({ attribute: false })
  agentPromptTemplate: AgentPromptTemplate | undefined;

  @property({ attribute: false })
  synchronization: PromptSynchronization = "preserve-local-draft";

  @property({ attribute: false })
  feedback: SettingsFeedbackMessage | undefined;

  @property({ type: Boolean })
  disabled = false;

  @state()
  private draft: AgentPromptTemplate | undefined;

  @state()
  private dirty = false;

  @state()
  private editorLayer: PromptEditorLayer = "shared";

  @state()
  private requestMode: AgentPromptPreviewMode = "full_projection";

  @state()
  private variableAnnouncement = "";

  private acceptedSynchronization = false;
  private acceptsNextAuthoritativeChange = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected willUpdate(changed: PropertyValues<this>): void {
    if (changed.has("synchronization") && this.synchronization === "preserve-local-draft") {
      this.acceptedSynchronization = false;
      this.acceptsNextAuthoritativeChange = false;
    }
    const acceptSynchronization =
      this.synchronization === "accept-parent-value" && !this.acceptedSynchronization;
    const previous = changed.get("agentPromptTemplate") as AgentPromptTemplate | undefined;
    const authoritativeChanged =
      changed.has("agentPromptTemplate") &&
      !agentPromptTemplatesEqual(previous, this.agentPromptTemplate);
    const acceptDelayedAuthoritativeChange =
      this.synchronization === "accept-parent-value" &&
      this.acceptsNextAuthoritativeChange &&
      authoritativeChanged;
    const acceptParent = acceptSynchronization || acceptDelayedAuthoritativeChange;
    if (!acceptParent && !authoritativeChanged) return;
    const authoritative = this.agentPromptTemplate;
    if (!authoritative) {
      if (!this.dirty) this.draft = undefined;
      return;
    }
    if (acceptSynchronization) {
      this.acceptedSynchronization = true;
      this.acceptsNextAuthoritativeChange = !authoritativeChanged;
    } else if (acceptDelayedAuthoritativeChange) {
      this.acceptsNextAuthoritativeChange = false;
    }
    if (
      acceptParent ||
      !this.draft ||
      !this.dirty ||
      agentPromptTemplatesEqual(authoritative, this.draft)
    ) {
      this.draft = cloneAgentPromptTemplate(authoritative);
      this.dirty = false;
    }
  }

  protected render() {
    const template = this.draft;
    const activeSection = this.activeSection();
    const descriptor = promptSectionDescriptor(activeSection);
    const errors = template ? validateAgentPromptTemplate(template) : {};
    const invalid = Object.keys(errors).length > 0;

    return html`
      <span class="prompt-draft-status" data-dirty=${this.dirty ? "true" : "false"}>
        ${template ? (this.dirty ? "Unsaved Changes" : "Saved") : nothing}
      </span>

      ${renderSettingsFeedback(this.feedback)}
      ${
        !template
          ? html`<p class="settings-empty-state">Loading the Agent prompt template…</p>`
          : html`
              ${this.renderComposition()}
              ${this.renderEditor(
                template,
                activeSection,
                descriptor,
                errors[activeSection],
                invalid,
              )}
              ${this.renderPreview(template, errors)}
            `
      }
    `;
  }

  private renderComposition() {
    const request = promptRequestSectionDescriptor(this.requestMode);
    return html`
      <section class="prompt-composition" aria-labelledby="prompt-composition-heading">
        <div class="prompt-composition-heading-row">
          <div>
            <h2 id="prompt-composition-heading">Prompt Composition</h2>
            <p>Choose either input layer to edit. The rendered prompt always combines both.</p>
          </div>
          <label class="prompt-request-mode" for="prompt-request-mode">
            <span>Request type</span>
            <select
              id="prompt-request-mode"
              .value=${this.requestMode}
              @change=${this.changeRequestMode}
            >
              ${REQUEST_PROMPT_SECTIONS.map(
                ({ key, title }) => html`<option value=${key}>${title}</option>`,
              )}
            </select>
          </label>
        </div>

        <div class="prompt-composition-flow">
          <label
            class="prompt-composition-part"
            data-active=${this.editorLayer === "shared" ? "true" : "false"}
          >
            <span class="prompt-composition-title">
              <input
                type="radio"
                name="prompt-editor-layer"
                value="shared"
                .checked=${this.editorLayer === "shared"}
                @change=${this.changeEditorLayer}
              />
              <strong>Shared Instructions</strong>
            </span>
            <span>Always included · contains the request instruction tag</span>
          </label>
          <span class="prompt-composition-operator" aria-hidden="true">+</span>
          <label
            class="prompt-composition-part"
            data-active=${this.editorLayer === "request" ? "true" : "false"}
          >
            <span class="prompt-composition-title">
              <input
                type="radio"
                name="prompt-editor-layer"
                value="request"
                .checked=${this.editorLayer === "request"}
                @change=${this.changeEditorLayer}
              />
              <strong>Request Instructions</strong>
            </span>
            <span>One variant included · ${request.title}</span>
          </label>
          <span class="prompt-composition-operator" aria-hidden="true">=</span>
          <div class="prompt-composition-result">
            <strong>Rendered Prompt</strong>
            <span>Exact composed output · updates automatically</span>
          </div>
        </div>
      </section>
    `;
  }

  private renderEditor(
    template: AgentPromptTemplate,
    section: PromptTemplateSection,
    descriptor: PromptSectionDescriptor,
    error: string | undefined,
    invalid: boolean,
  ) {
    const value = template[section];
    const length = [...value].length;
    const occurrences = templatePlaceholderOccurrences(value);
    return html`
      <section class="prompt-editor-section" aria-labelledby="prompt-editor-heading">
        <header class="prompt-editor-header">
          <div>
            <span>${descriptor.layer === "shared" ? "Shared layer" : "Request layer"}</span>
            <h2 id="prompt-editor-heading">${descriptor.title}</h2>
            <p id="prompt-editor-description">${descriptor.description}</p>
          </div>
        </header>

        <form class="prompt-form" @submit=${this.save}>
          <div class="prompt-variable-bar" aria-label="Insert template variables">
            <span>Insert variable</span>
            ${
              descriptor.variables.length
                ? descriptor.variables.map((variable) => {
                    const count = occurrences.filter(({ name }) => name === variable.name).length;
                    return html`<button
                      type="button"
                      class="prompt-variable-token"
                      data-variable=${variable.name}
                      data-state=${count === 0 ? "available" : count === 1 ? "inserted" : "duplicate"}
                      aria-label=${
                        count === 0
                          ? `Insert ${variable.label} ${variable.token}`
                          : `Select ${variable.label} ${variable.token}`
                      }
                      title=${variable.description}
                      @click=${() => this.insertVariable(variable)}
                      ?disabled=${this.disabled || !this.agentPromptTemplate}
                    >
                      <span>${variable.label}</span>
                      <code>${variable.token}</code>
                      <span class="prompt-variable-state">
                        ${count === 0 ? "Insert" : count === 1 ? "Inserted" : `${count} used`}
                      </span>
                    </button>`;
                  })
                : html`<span class="prompt-no-variables">None for this request type</span>`
            }
          </div>

          <label class="visually-hidden" for="prompt-editor">${descriptor.title}</label>
          <textarea
            id="prompt-editor"
            class="prompt-editor"
            aria-describedby="prompt-editor-description prompt-editor-validation"
            aria-invalid=${error ? "true" : "false"}
            required
            .value=${value}
            @input=${this.edit}
            ?disabled=${this.disabled || !this.agentPromptTemplate}
          ></textarea>
          <p class="visually-hidden" role="status" aria-live="polite">
            ${this.variableAnnouncement}
          </p>
          <div class="prompt-editor-meta">
            <p id="prompt-editor-validation" class=${error ? "prompt-validation error" : "help"}>
              ${
                error ??
                "The complete template is saved atomically. Use {{ and }} for literal braces."
              }
            </p>
            <span
              class="prompt-character-count"
              data-over-limit=${length > MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS ? "true" : "false"}
            >
              ${length.toLocaleString()} /
              ${MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS.toLocaleString()}
            </span>
          </div>
          <div class="prompt-actions">
            <button
              type="button"
              @click=${this.revert}
              ?disabled=${this.disabled || !this.dirty || !this.agentPromptTemplate}
            >
              Revert Changes
            </button>
            <span class="prompt-action-spacer"></span>
            <button
              type="button"
              @click=${() => this.emit({ type: "reset" })}
              ?disabled=${this.disabled || !this.agentPromptTemplate}
            >
              Reset All to Default…
            </button>
            <button
              type="submit"
              class="primary"
              ?disabled=${this.disabled || !this.dirty || invalid}
            >
              Save Template
            </button>
          </div>
        </form>
      </section>
    `;
  }

  private renderPreview(
    template: AgentPromptTemplate,
    errors: ReturnType<typeof validateAgentPromptTemplate>,
  ) {
    const invalid = Object.keys(errors).length > 0;
    const request = promptRequestSectionDescriptor(this.requestMode);
    return html`
      <section class="prompt-preview-section" aria-labelledby="prompt-preview-heading">
        <header class="prompt-preview-header">
          <div class="prompt-preview-heading">
            <h2 id="prompt-preview-heading">Rendered Prompt</h2>
            <span>Shared Instructions + ${request.title}</span>
          </div>
          <span class="prompt-preview-status"
            >${invalid ? "Needs attention" : `Schema ${template.schema_version}`}</span
          >
        </header>
        <div class="prompt-preview-content">
          ${
            invalid
              ? html`<div class="prompt-preview-errors" role="alert">
                  <p>Resolve these template errors before saving:</p>
                  <ul>
                    ${Object.values(errors).map((error) => html`<li>${error}</li>`)}
                  </ul>
                </div>`
              : nothing
          }
          <pre
            class="prompt-preview-output"
            aria-label="Rendered Agent instruction"
          ><code>${renderAgentPromptTemplate(template, this.requestMode)}</code></pre>
          <aside class="prompt-transport-note">
            Lens sends this instruction followed by canonical JSON and any linked image blocks.
            Those structured transport blocks contain observations; Lens does not append hidden
            prompt prose.
          </aside>
        </div>
      </section>
    `;
  }

  private activeSection(): PromptTemplateSection {
    return this.editorLayer === "shared" ? "common" : this.requestMode;
  }

  private changeEditorLayer = (event: Event): void => {
    this.editorLayer = (event.currentTarget as HTMLInputElement).value as PromptEditorLayer;
    this.variableAnnouncement = "";
  };

  private changeRequestMode = (event: Event): void => {
    this.requestMode = (event.currentTarget as HTMLSelectElement).value as AgentPromptPreviewMode;
    this.variableAnnouncement = "";
  };

  private edit = (event: Event): void => {
    this.updateDraft(this.activeSection(), (event.currentTarget as HTMLTextAreaElement).value);
  };

  private insertVariable(variable: PromptVariableDescriptor): void {
    const editor = this.querySelector<HTMLTextAreaElement>("#prompt-editor");
    if (!editor || !this.draft) return;
    const occurrences = templatePlaceholderOccurrences(editor.value).filter(
      ({ name }) => name === variable.name,
    );
    const first = occurrences[0];
    if (first) {
      this.variableAnnouncement =
        occurrences.length === 1
          ? `${variable.label} selected.`
          : `Selected the first of ${occurrences.length} ${variable.label} variables.`;
      this.focusEditorSelection(first.start, first.end);
      return;
    }

    const start = editor.selectionStart;
    const end = editor.selectionEnd;
    editor.setRangeText(variable.token, start, end, "end");
    const caret = editor.selectionStart;
    this.updateDraft(this.activeSection(), editor.value);
    this.variableAnnouncement = `${variable.label} inserted.`;
    this.focusEditorSelection(caret, caret);
  }

  private updateDraft(section: PromptTemplateSection, value: string): void {
    if (!this.draft) return;
    this.draft = { ...this.draft, [section]: value };
    this.dirty = !agentPromptTemplatesEqual(this.draft, this.agentPromptTemplate);
  }

  private focusEditorSelection(start: number, end: number): void {
    void this.updateComplete.then(() => {
      const editor = this.querySelector<HTMLTextAreaElement>("#prompt-editor");
      editor?.focus();
      editor?.setSelectionRange(start, end);
    });
  }

  private revert = (): void => {
    if (!this.agentPromptTemplate) return;
    this.draft = cloneAgentPromptTemplate(this.agentPromptTemplate);
    this.dirty = false;
    this.variableAnnouncement = "All prompt sections reverted.";
  };

  private save = (event: SubmitEvent): void => {
    event.preventDefault();
    if (!this.dirty || !this.draft) return;
    if (Object.keys(validateAgentPromptTemplate(this.draft)).length > 0) return;
    this.emit({ type: "save", agentPromptTemplate: cloneAgentPromptTemplate(this.draft) });
  };

  private emit(intent: PromptIntent): void {
    dispatchComponentEvent(this, PROMPT_INTENT_EVENT, intent);
  }
}
