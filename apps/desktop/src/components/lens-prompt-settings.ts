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
  type PromptSectionDescriptor,
  type PromptTemplateSection,
  type PromptVariableDescriptor,
} from "../agent-prompt-template";
import type { PromptSynchronization, SettingsFeedbackMessage } from "../application/view-models";
import type { AgentPromptTemplate, PromptPresetCollection, PromptPreset } from "../types";
import { dispatchComponentEvent, PROMPT_INTENT_EVENT, type PromptIntent } from "./events";
import { renderSettingsFeedback } from "./settings-feedback";

@customElement("lens-prompt-settings")
export class LensPromptSettings extends LitElement {
  @property({ attribute: false })
  agentPromptTemplate: AgentPromptTemplate | undefined;

  @property({ attribute: false })
  promptPresets: PromptPresetCollection | undefined;

  @state() private editingId = "";
  @state() private draftName = "";
  private basePreset: PromptPreset | undefined;
  private readonly retainedDrafts = new Map<
    string,
    { base: PromptPreset; template: AgentPromptTemplate; name: string }
  >();

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
  private requestMode: AgentPromptPreviewMode = "full_projection";

  @state()
  private variableAnnouncement = "";

  private acceptedSynchronization = false;
  private acceptsNextAuthoritativeChange = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected willUpdate(changed: PropertyValues<this>): void {
    if (this.promptPresets) {
      if (!changed.has("promptPresets") && this.editingId) return;
      const current = this.promptPresets.presets.find((preset) => preset.id === this.editingId);
      if (!this.editingId) this.loadPreset(this.promptPresets.selected_id);
      else if (current && (!this.dirty || this.matchesDraft(current))) {
        this.retainedDrafts.delete(current.id);
        this.loadPreset(current.id);
      } else if (!current && !this.dirty) this.loadPreset(this.promptPresets.selected_id);
      return;
    }
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
    const errors = template ? validateAgentPromptTemplate(template) : {};
    const invalid = Object.keys(errors).length > 0;
    const editor = !template
      ? html`<p class="settings-empty-state">Loading the Agent prompt template…</p>`
      : html` ${this.renderEditor(template, "common", { ...promptSectionDescriptor("common"), title: "Instructions" }, errors.common)}
          ${Object.entries(errors).some(([key]) => key !== "common") ? html`<p class="error" role="alert">A request template needs attention. Open Advanced Prompt Settings to review the validation errors before saving.</p>` : nothing}
          <details class="settings-disclosure prompt-advanced">
            <summary>Advanced Prompt Settings</summary>
            <label class="prompt-request-mode" for="prompt-request-mode">
              <span>Request type</span>
              <select
                id="prompt-request-mode"
                .value=${this.requestMode}
                @change=${this.changeRequestMode}
              >
                ${REQUEST_PROMPT_SECTIONS.map(({ key, title }) => html`<option value=${key}>${title}</option>`)}
              </select>
            </label>
            ${this.renderEditor(template, this.requestMode, { ...promptSectionDescriptor(this.requestMode), title: "Request instructions" }, errors[this.requestMode], true)}
          </details>
          <details class="settings-disclosure prompt-preview">
            <summary>Prompt Preview</summary>
            ${this.renderPreview(template, errors)}
          </details>
          <div class="prompt-actions prompt-save-actions">
            <button
              type="button"
              @click=${this.revert}
              ?hidden=${Boolean(this.promptPresets)}
              ?disabled=${this.disabled || !this.dirty}
            >
              Revert Changes
            </button>
            <button
              type="button"
              ?hidden=${Boolean(this.promptPresets)}
              @click=${() => this.emit({ type: "reset" })}
              ?disabled=${this.disabled}
            >
              Reset All to Default…
            </button>
            <span
              class="prompt-draft-status"
              data-dirty=${this.dirty ? "true" : "false"}
              role="status"
              >${this.dirty ? "Unsaved Changes" : "Saved"}</span
            >
            <span class="prompt-action-spacer"></span>
            <button
              type="submit"
              class="primary"
              ?disabled=${this.disabled || !this.dirty || invalid || (Boolean(this.promptPresets) && (!this.validMetadata() || !this.currentPreset()))}
            >
              ${this.promptPresets ? "Save Preset" : "Save Template"}
            </button>
          </div>`;
    return html`
      ${renderSettingsFeedback(this.feedback)}
      ${this.promptPresets ? this.renderPresets(editor) : html`<form class="settings-group prompt-presets" @submit=${this.save}>${editor}</form>`}
    `;
  }

  private renderEditor(
    template: AgentPromptTemplate,
    section: PromptTemplateSection,
    descriptor: PromptSectionDescriptor,
    error: string | undefined,
    advanced = false,
  ) {
    const value = template[section];
    const length = [...value].length;
    const occurrences = templatePlaceholderOccurrences(value);
    return html`
      <section
        class="prompt-editor-section"
        aria-labelledby=${advanced ? "prompt-request-editor-heading" : "prompt-editor-heading"}
      >
        <header class="prompt-editor-header">
          <div>
            <h2 id=${advanced ? "prompt-request-editor-heading" : "prompt-editor-heading"}>
              ${descriptor.title}
            </h2>
            <p
              class="visually-hidden"
              id=${advanced ? "prompt-request-editor-description" : "prompt-editor-description"}
            >
              ${descriptor.description}
            </p>
          </div>
        </header>

        <div class="prompt-form">
          <label class="visually-hidden" for=${advanced ? "prompt-request-editor" : "prompt-editor"}
            >${descriptor.title}</label
          >
          <textarea
            id=${advanced ? "prompt-request-editor" : "prompt-editor"}
            class="prompt-editor"
            aria-describedby=${advanced ? "prompt-request-editor-description prompt-request-editor-validation" : "prompt-editor-description prompt-editor-validation"}
            aria-invalid=${error ? "true" : "false"}
            rows=${advanced ? 3 : 7}
            required
            .value=${value}
            @input=${(event: Event) => this.updateDraft(section, (event.currentTarget as HTMLTextAreaElement).value)}
            ?disabled=${this.disabled || !this.agentPromptTemplate}
          ></textarea>
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
                      @click=${() => this.insertVariable(variable, section, advanced)}
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
          <p class="visually-hidden" role="status" aria-live="polite">
            ${this.variableAnnouncement}
          </p>
          <div class="prompt-editor-meta">
            <p
              id=${advanced ? "prompt-request-editor-validation" : "prompt-editor-validation"}
              class=${error ? "prompt-validation error" : "help"}
            >
              ${error ?? "Use {{ and }} for literal braces."}
            </p>
            <span
              class="prompt-character-count"
              data-over-limit=${length > MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS ? "true" : "false"}
            >
              ${length.toLocaleString()} /
              ${MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS.toLocaleString()}
            </span>
          </div>
        </div>
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
          <span class="prompt-preview-status">${invalid ? "Needs attention" : ""}</span>
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

  private currentPreset(): PromptPreset | undefined {
    return this.promptPresets?.presets.find((preset) => preset.id === this.editingId);
  }

  private matchesDraft(preset: PromptPreset): boolean {
    return preset.name === this.draftName && agentPromptTemplatesEqual(preset.template, this.draft);
  }

  private updateDirty(): void {
    this.dirty = this.promptPresets
      ? Boolean(this.basePreset && !this.matchesDraft(this.basePreset))
      : !agentPromptTemplatesEqual(this.draft, this.agentPromptTemplate);
  }

  private loadPreset(id: string): void {
    const preset = this.promptPresets?.presets.find((item) => item.id === id);
    if (!preset) return;
    this.editingId = id;
    this.basePreset = preset;
    this.draft = cloneAgentPromptTemplate(preset.template);
    this.draftName = preset.name;
    this.dirty = false;
  }

  private changePreset = (event: Event): void => {
    if (this.dirty && this.basePreset && this.draft)
      this.retainedDrafts.set(this.editingId, {
        base: this.basePreset,
        template: this.draft,
        name: this.draftName,
      });
    else this.retainedDrafts.delete(this.editingId);
    const id = (event.currentTarget as HTMLSelectElement).value;
    this.loadPreset(id);
    const retained = this.retainedDrafts.get(id);
    if (retained) {
      this.editingId = id;
      this.basePreset = retained.base;
      this.draft = retained.template;
      this.draftName = retained.name;
      this.updateDirty();
    }
  };

  private renderPresets(editor: ReturnType<typeof html>) {
    const collection = this.promptPresets!;
    const preset = this.currentPreset();
    const selected = collection.presets.find((item) => item.id === collection.selected_id);
    const conflict = this.dirty && (!preset || preset.revision !== this.basePreset?.revision);
    return html` <div class="prompt-preset-active">
        <span role="status">In use: <strong>${selected?.name}</strong></span>
        <button
          type="button"
          ?disabled=${this.disabled || !preset || preset.id === collection.selected_id}
          @click=${() => preset && this.emit({ type: "presets", change: { type: "select", id: preset.id } })}
        >
          Use This Preset
        </button>
      </div>
      <form
        class="settings-group prompt-presets"
        aria-labelledby="prompt-presets-heading"
        @submit=${this.save}
      >
        <div class="prompt-preset-heading">
          <h2 id="prompt-presets-heading">Presets</h2>
          <div class="prompt-actions">
            <button
              type="button"
              ?disabled=${this.disabled || !this.draft || collection.presets.length >= 64 || Object.keys(validateAgentPromptTemplate(this.draft)).length > 0}
              @click=${() => this.duplicatePreset()}
            >
              Duplicate
            </button>
            <button
              type="button"
              ?disabled=${this.disabled || !preset || collection.presets.length <= 1}
              @click=${() => {
                if (preset)
                  this.emit({
                    type: "presets",
                    change: {
                      type: "delete",
                      id: preset.id,
                      expected_revision: this.basePreset!.revision,
                    },
                  });
              }}
            >
              Delete…
            </button>
          </div>
        </div>
        <label
          >Preset to edit
          <select
            id="prompt-preset-list"
            .value=${this.editingId}
            @change=${this.changePreset}
            ?disabled=${this.disabled}
          >
            ${!preset && !this.retainedDrafts.has(this.editingId) ? html`<option value=${this.editingId} .selected=${true}>Deleted preset (unsaved draft)</option>` : nothing}
            ${[...this.retainedDrafts.entries()].filter(([id]) => !collection.presets.some((item) => item.id === id)).map(([id, retained]) => html`<option value=${id} .selected=${id === this.editingId}>${retained.name} · Deleted, unsaved draft</option>`)}
            ${collection.presets.map((item) => html`<option value=${item.id} .selected=${item.id === this.editingId}>${item.name}${item.id === collection.selected_id ? " · In use" : ""}</option>`)}
          </select>
        </label>
        <label
          >Name
          <input
            id="prompt-preset-name"
            class="prompt-preset-field"
            type="text"
            maxlength="80"
            .value=${this.draftName}
            ?disabled=${this.disabled}
            @input=${(event: Event) => {
              this.draftName = (event.currentTarget as HTMLInputElement).value;
              this.updateDirty();
            }}
        /></label>
        ${!this.validMetadata() ? html`<p role="alert">Enter a name of 1–80 characters without control characters</p>` : nothing}
        ${
          conflict
            ? html`<p role="alert">
                  This preset was changed or deleted elsewhere. Your draft is preserved. Duplicate
                  it to keep your changes, or reload the latest saved preset.
                </p>
                <button type="button" ?disabled=${this.disabled} @click=${this.revert}>
                  Reload Saved Preset
                </button>`
            : nothing
        }
        ${editor}
      </form>
      <div class="prompt-preset-reset">
        <button
          type="button"
          ?disabled=${this.disabled}
          @click=${() => this.emit({ type: "presets", change: { type: "reset_all", expected_catalog_revision: collection.revision } })}
        >
          Reset All Presets…
        </button>
      </div>`;
  }

  private validMetadata(): boolean {
    return (
      Boolean(this.draftName.trim()) &&
      [...this.draftName].length <= 80 &&
      !/\p{Cc}/u.test(this.draftName)
    );
  }

  openCreatedPreset(collection: PromptPresetCollection, id: string): void {
    if (this.dirty && this.basePreset && this.draft)
      this.retainedDrafts.set(this.editingId, {
        base: this.basePreset,
        template: this.draft,
        name: this.draftName,
      });
    if (!this.promptPresets || collection.revision >= this.promptPresets.revision)
      this.promptPresets = collection;
    this.loadPreset(id);
  }

  acceptResetPresets(collection: PromptPresetCollection): void {
    this.retainedDrafts.clear();
    if (!this.promptPresets || collection.revision >= this.promptPresets.revision)
      this.promptPresets = collection;
    this.loadPreset(this.promptPresets.selected_id);
  }

  private duplicatePreset(): void {
    const template = this.draft;
    if (
      !template ||
      (this.promptPresets?.presets.length ?? 0) >= 64 ||
      Object.keys(validateAgentPromptTemplate(template)).length > 0
    )
      return;
    this.emit({
      type: "presets",
      change: {
        type: "create",
        name: `${[...this.draftName].slice(0, 75).join("")} Copy`,
        template: cloneAgentPromptTemplate(template),
      },
    });
  }

  private changeRequestMode = (event: Event): void => {
    this.requestMode = (event.currentTarget as HTMLSelectElement).value as AgentPromptPreviewMode;
    this.variableAnnouncement = "";
  };

  private insertVariable(
    variable: PromptVariableDescriptor,
    section: PromptTemplateSection,
    advanced: boolean,
  ): void {
    const selector = advanced ? "#prompt-request-editor" : "#prompt-editor";
    const editor = this.querySelector<HTMLTextAreaElement>(selector);
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
      this.focusEditorSelection(first.start, first.end, selector);
      return;
    }

    const start = editor.selectionStart;
    const end = editor.selectionEnd;
    editor.setRangeText(variable.token, start, end, "end");
    const caret = editor.selectionStart;
    this.updateDraft(section, editor.value);
    this.variableAnnouncement = `${variable.label} inserted.`;
    this.focusEditorSelection(caret, caret, selector);
  }

  private updateDraft(section: PromptTemplateSection, value: string): void {
    if (!this.draft) return;
    this.draft = { ...this.draft, [section]: value };
    this.updateDirty();
  }

  private focusEditorSelection(start: number, end: number, selector: string): void {
    void this.updateComplete.then(() => {
      const editor = this.querySelector<HTMLTextAreaElement>(selector);
      editor?.focus();
      editor?.setSelectionRange(start, end);
    });
  }

  private revert = (): void => {
    if (this.promptPresets) {
      this.retainedDrafts.delete(this.editingId);
      this.loadPreset(this.currentPreset()?.id ?? this.promptPresets.selected_id);
      return;
    }
    if (!this.agentPromptTemplate) return;
    this.draft = cloneAgentPromptTemplate(this.agentPromptTemplate);
    this.dirty = false;
    this.variableAnnouncement = "All prompt sections reverted.";
  };

  private save = (event: SubmitEvent): void => {
    event.preventDefault();
    if (!this.dirty || !this.draft) return;
    if (Object.keys(validateAgentPromptTemplate(this.draft)).length > 0) return;
    if (this.promptPresets && this.basePreset) {
      if (!this.validMetadata() || !this.currentPreset()) return;
      this.emit({
        type: "presets",
        change: {
          type: "update",
          id: this.basePreset.id,
          expected_revision: this.basePreset.revision,
          name: this.draftName,
          template: cloneAgentPromptTemplate(this.draft),
        },
      });
    } else this.emit({ type: "save", agentPromptTemplate: cloneAgentPromptTemplate(this.draft) });
  };

  private emit(intent: PromptIntent): void {
    dispatchComponentEvent(this, PROMPT_INTENT_EVENT, intent);
  }
}
