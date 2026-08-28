import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import fontAwesomeStyles from "@fortawesome/fontawesome-free/css/fontawesome.css?inline";
import fontAwesomeSolidStyles from "@fortawesome/fontawesome-free/css/solid.css?inline";
import { LitElement, css, html, nothing, unsafeCSS } from "lit";
import { customElement, state } from "lit/decorators.js";
import componentStyles from "./styles.css?inline";
import { externalMarkdownUrl } from "./markdown";
import "./streaming-markdown";
import type { StreamingMarkdownState } from "./streaming-markdown";
import type {
  AgentKind,
  AgentRuntimeState,
  AgentSelectionState,
  AppConfig,
  AppSnapshot,
  LensOutputBlock,
  LensState,
} from "./types";
import {
  AGENT_RUNTIME_LABEL,
  AGENT_SELECTION_LABEL,
  imageDataUrl,
  inputMediaPreviewUrl,
  isAgentRuntimeActive,
  lensOutputBlocks,
  lensSourceJson,
  selectedAgent,
  showsLensProgress,
  shouldApplySnapshot,
  STAGE_LABEL,
  supportedAuthMethods,
} from "./view-model";

type LensTab = "translation" | "source";

@customElement("lens-app")
export class LensApp extends LitElement {
  static styles = [
    css`
      :host {
        display: block;
        min-height: 100%;
      }
    `,
    unsafeCSS(componentStyles),
    unsafeCSS(fontAwesomeStyles),
    unsafeCSS(fontAwesomeSolidStyles),
  ];

  @state()
  private config: AppConfig | undefined = undefined;

  @state()
  private agentSelection: AgentSelectionState = { stage: "unselected", auth_methods: [] };

  @state()
  private agentRuntime: AgentRuntimeState = {
    stage: "not_installed",
    downloaded_bytes: 0,
  };

  @state()
  private lens: LensState = { stage: "idle", output_blocks: [] };

  @state()
  private trusted = false;

  @state()
  private busy = false;

  @state()
  private message = "";

  @state()
  private activeLensTab: LensTab = "translation";

  @state()
  private activeInputMediaIndex = 0;

  @state()
  private inputMediaPreviewError = "";

  @state()
  private responsePromptDraft = "";

  @state()
  private responsePromptDirty = false;

  private unlisten: UnlistenFn[] = [];
  private permissionTimer: number | undefined = undefined;
  private revision = -1;
  private loadGeneration = 0;

  connectedCallback(): void {
    super.connectedCallback();
    const generation = ++this.loadGeneration;
    void this.load(generation);
    if (this.view === "settings") {
      this.permissionTimer = window.setInterval(() => void this.refreshPermission(), 2_000);
    }
  }

  disconnectedCallback(): void {
    this.loadGeneration += 1;
    for (const unlisten of this.unlisten) unlisten();
    this.unlisten = [];
    if (this.permissionTimer !== undefined) window.clearInterval(this.permissionTimer);
    this.permissionTimer = undefined;
    super.disconnectedCallback();
  }

  private get view(): "settings" | "overlay" | "target-selection" {
    const view = new URLSearchParams(window.location.search).get("view");
    if (view === "overlay" || view === "target-selection") return view;
    return "settings";
  }

  private async load(generation: number): Promise<void> {
    try {
      this.message = "Subscribing to application state…";
      const unlisten = await listen<AppSnapshot>("app-state-changed", ({ payload }) => {
        this.applySnapshot(payload);
      });
      if (generation !== this.loadGeneration || !this.isConnected) {
        unlisten();
        return;
      }
      this.unlisten.push(unlisten);

      this.message = "Loading application state…";
      this.applySnapshot(await invoke<AppSnapshot>("get_app_snapshot"));
      if (generation !== this.loadGeneration || !this.isConnected) return;

      if (this.view !== "settings") {
        this.message = "";
        return;
      }
      this.message = "Checking Accessibility permission…";
      this.trusted = await invoke<boolean>("accessibility_permission");
      if (generation === this.loadGeneration && this.isConnected) this.message = "";
    } catch (error) {
      if (generation === this.loadGeneration && this.isConnected) this.message = String(error);
    }
  }

  private applySnapshot(next: AppSnapshot): void {
    if (!shouldApplySnapshot(this.revision, next.revision)) return;
    this.revision = next.revision;
    if (!this.responsePromptDirty || next.config.response_prompt === this.responsePromptDraft) {
      this.responsePromptDraft = next.config.response_prompt;
      this.responsePromptDirty = false;
    }
    this.config = next.config;
    this.agentSelection = next.agent_selection;
    this.agentRuntime = next.agent_runtime;
    this.applyLensState(next.lens);
  }

  protected render() {
    switch (this.view) {
      case "settings":
        return this.renderSettings();
      case "overlay":
        return this.renderOverlay();
      case "target-selection":
        return this.renderTargetSelection();
    }
  }

  private renderSettings() {
    return html`
      <main class="settings-shell" aria-label="Settings">
        <section class="settings-group" aria-labelledby="agent-heading">
          <h2 id="agent-heading">AI Agent</h2>
          <fieldset
            ?disabled=${
              this.busy ||
              !this.config ||
              isAgentRuntimeActive(this.agentRuntime.stage) ||
              ["checking", "authenticating", "signing_out"].includes(this.agentSelection.stage)
            }
          >
            <legend class="visually-hidden">AI agent to use</legend>
            ${this.agentOption("claude", "Claude")} ${this.agentOption("codex", "Codex")}
          </fieldset>
          <p class="help">The ACP agent, not Lens, manages authentication credentials.</p>
          ${this.renderAgentRuntimeStatus()}
          <output
            class=${this.agentSelection.stage === "selected" ? "status-ok" : "status-warning"}
          >
            ${
              this.agentSelection.error ??
              this.agentSelection.message ??
              AGENT_SELECTION_LABEL[this.agentSelection.stage]
            }
          </output>
          ${this.renderAgentSelectionAuthentication()} ${this.renderSelectedAgentActions()}
        </section>

        <section class="settings-group" aria-labelledby="prompt-heading">
          <h2 id="prompt-heading">Agent Prompt</h2>
          <form @submit=${this.saveResponsePrompt}>
            <textarea
              class="prompt-editor"
              aria-label="Agent Prompt"
              required
              .value=${this.responsePromptDraft}
              @input=${this.editResponsePrompt}
              ?disabled=${this.busy || !this.config}
            ></textarea>
            <p class="help">
              Controls how the Agent transforms the source. Lens appends fixed source-data
              boundaries and safety instructions when it sends the prompt.
            </p>
            <div class="prompt-actions">
              <button
                type="button"
                @click=${this.resetResponsePrompt}
                ?disabled=${this.busy || !this.config}
              >
                Reset to Default…
              </button>
              <button
                type="submit"
                class="primary"
                ?disabled=${this.busy || !this.responsePromptDirty || !this.responsePromptDraft.trim()}
              >
                Save Prompt
              </button>
            </div>
          </form>
        </section>

        <section class="settings-group" aria-labelledby="cwd-heading">
          <h2 id="cwd-heading">Working Directory</h2>
          <div class="directory-row">
            <input
              type="text"
              class="directory-field"
              aria-label="Working Directory"
              readonly
              .value=${this.config?.working_directory ?? ""}
            />
            <button @click=${this.chooseDirectory} ?disabled=${this.busy}>Choose…</button>
          </div>
          <p class="help">
            The default is your home directory. The agent uses this directory as the cwd for
            resolving its own project instructions and memory.
          </p>
        </section>

        <section class="settings-group" aria-labelledby="permission-heading">
          <h2 id="permission-heading">Accessibility</h2>
          <div class="permission-row">
            <output class=${this.trusted ? "status-ok" : "status-warning"}>
              ${this.trusted ? "Allowed" : "Permission required"}
            </output>
            ${
              this.trusted
                ? nothing
                : html`<button @click=${this.requestPermission}>Open System Settings</button>`
            }
          </div>
        </section>

        <footer>
          <span role="status">${this.message || STAGE_LABEL[this.lens.stage]}</span>
        </footer>
      </main>
    `;
  }

  private renderAgentRuntimeStatus() {
    const runtime = this.agentRuntime;
    const total = runtime.total_bytes;
    const showProgress = runtime.stage === "downloading";
    return html`
      <div class="runtime-status" aria-live="polite">
        <output class=${runtime.stage === "failed" ? "status-warning" : "runtime-message"}>
          ${runtime.error ?? runtime.message ?? AGENT_RUNTIME_LABEL[runtime.stage]}
        </output>
        ${
          showProgress
            ? total === undefined
              ? html`<progress aria-label="Agent runtime download progress"></progress>`
              : html`<progress
                  aria-label="Agent runtime download progress"
                  .value=${runtime.downloaded_bytes}
                  max=${total}
                ></progress>`
            : nothing
        }
      </div>
    `;
  }

  private agentOption(value: AgentKind, label: string) {
    return html`
      <label class="radio-row">
        <input
          type="radio"
          name="agent"
          value=${value}
          .checked=${selectedAgent(this.agentSelection) === value}
          @change=${() => this.setAgent(value)}
        />
        <span>${label}</span>
      </label>
    `;
  }

  private renderAgentSelectionAuthentication() {
    if (this.agentSelection.stage !== "authentication_required") return nothing;
    const methods = this.agentSelection.auth_methods.filter((method) => method.supported);
    return html`
      <div class="agent-actions" aria-label="Agent authentication">
        ${
          methods.length
            ? methods.map(
                (method) => html`
                  <button @click=${() => this.authenticateAgentSelection(method.id)}>
                    Authenticate with ${method.name}…
                  </button>
                `,
              )
            : html`<p>Authenticate with this Agent's existing CLI, then select it again.</p>`
        }
      </div>
    `;
  }

  private renderSelectedAgentActions() {
    const agent = selectedAgent(this.agentSelection);
    if (!agent) return nothing;
    const label = agent === "claude" ? "Claude" : "Codex";
    return html`
      <div class="agent-actions" aria-label="${label} authentication management">
        <button @click=${this.reauthenticateAgentSelection} ?disabled=${this.busy}>
          Reauthenticate…
        </button>
        <button @click=${this.signOutAgentSelection} ?disabled=${this.busy}>Sign Out…</button>
      </div>
    `;
  }

  private renderTargetSelection() {
    const selection = this.lens.selection;
    const items = selection?.items ?? [];
    const pickerActive = selection?.stage === "picking";
    const operationAvailable = Boolean(this.lens.operation_id && selection);
    const canAdd = Boolean(
      operationAvailable && !pickerActive && items.length < (selection?.maximum_targets ?? 0),
    );
    const canEdit = Boolean(operationAvailable && !pickerActive && !this.busy);
    return html`
      <section class="target-selection-shell" aria-label="Selected windows">
        <header class="target-selection-toolbar">
          <output class="target-selection-count" aria-label="Selected window count">
            ${items.length}<span aria-hidden="true"> / ${selection?.maximum_targets ?? 0}</span>
          </output>
          <div class="target-selection-actions" aria-label="Selection actions">
            <button
              type="button"
              class="target-selection-icon-button"
              aria-label="Add another window"
              title="Add another window"
              ?disabled=${!canAdd || this.busy}
              @click=${this.addSelectionTarget}
            >
              <i class="fa-solid fa-plus" aria-hidden="true"></i>
            </button>
            <button
              type="button"
              class="target-selection-icon-button is-primary"
              aria-label="Use selected windows"
              title="Use selected windows"
              ?disabled=${!canEdit || items.length === 0}
              @click=${this.confirmSelectionTargets}
            >
              <i class="fa-solid fa-check" aria-hidden="true"></i>
            </button>
          </div>
        </header>

        <ol class="target-selection-list" aria-label="Window previews">
          ${items.map(
            (item) => html`
              <li class="target-selection-card">
                <div class="target-selection-image">
                  <div class="target-selection-placeholder" aria-hidden="true">
                    <i class="fa-solid fa-window-maximize"></i>
                  </div>
                  ${
                    item.preview_uri
                      ? html`<img
                          src=${item.preview_uri}
                          alt=${`Preview of ${item.window.application_name}${
                            item.window.title ? ` — ${item.window.title}` : ""
                          }`}
                          draggable="false"
                          @error=${(event: Event) => {
                            (event.currentTarget as HTMLImageElement).hidden = true;
                          }}
                        />`
                      : nothing
                  }
                  <button
                    type="button"
                    class="target-selection-remove"
                    aria-label=${`Remove ${item.window.application_name}${
                      item.window.title ? ` — ${item.window.title}` : ""
                    }`}
                    title="Remove window"
                    ?disabled=${!canEdit}
                    @click=${() => this.removeSelectionTarget(item.id)}
                  >
                    <i class="fa-solid fa-xmark" aria-hidden="true"></i>
                  </button>
                </div>
                <div class="target-selection-caption">
                  <strong>${item.window.application_name || "Application"}</strong>
                  <span title=${item.window.title || "Untitled window"}
                    >${item.window.title || "Untitled window"}</span
                  >
                </div>
                ${
                  item.preview_error
                    ? html`<span class="visually-hidden">Preview unavailable</span>`
                    : nothing
                }
              </li>
            `,
          )}
        </ol>

        <p class="visually-hidden" role="status" aria-live="polite">
          ${
            this.message ||
            selection?.notice ||
            (pickerActive ? "Choose one window in the system picker." : "")
          }
        </p>
      </section>
    `;
  }

  private renderOverlay() {
    const context = this.lens.context;
    const targets = this.lens.target_set?.targets ?? [];
    const outputBlocks = lensOutputBlocks(this.lens);
    const sourceJson = lensSourceJson(this.lens);
    const activeAgent = this.lens.agent;
    const authenticationMethods = supportedAuthMethods(this.lens);
    const firstWindow = targets[0]?.window;
    const applicationName =
      targets.length > 1 ? `${targets.length} Windows` : (firstWindow?.application_name ?? "Lens");
    const windowContext = targets.length
      ? targets
          .map(({ window }) =>
            window.title ? `${window.application_name} — ${window.title}` : window.application_name,
          )
          .join("\n")
      : applicationName;
    return html`
      <div class="overlay-shell">
        <header class="overlay-header" data-tauri-drag-region="deep">
          <h1 class="overlay-title" title=${windowContext}>
            <strong>${applicationName}</strong>
            ${
              targets.length === 1 && firstWindow?.title
                ? html`<span> — ${firstWindow.title}</span>`
                : targets.length > 1
                  ? html`<span> — Combined context</span>`
                  : nothing
            }
          </h1>
          <button
            type="button"
            class="close-button"
            data-tauri-drag-region="false"
            aria-label="Close Lens"
            @click=${this.closeWindow}
          >
            <span class="close-icon" aria-hidden="true"></span>
          </button>
        </header>

        <main class="overlay-main">
          ${this.message ? html`<p class="error" role="alert">${this.message}</p>` : nothing}
          ${this.lens.error ? html`<p class="error" role="alert">${this.lens.error}</p>` : nothing}
          ${
            activeAgent?.authentication_message
              ? html`<p class="notice" role="status">${activeAgent.authentication_message}</p>`
              : nothing
          }
          ${
            this.lens.stage === "authentication_required"
              ? html`
                  <section class="overlay-actions" aria-label="Agent authentication">
                    ${
                      authenticationMethods.length
                        ? authenticationMethods.map(
                            (method) => html`
                              <button @click=${() => this.authenticate(method.id)}>
                                Authenticate with ${method.name}…
                              </button>
                            `,
                          )
                        : html`<p>Authenticate with this agent's existing CLI, then try again.</p>`
                    }
                    <button @click=${this.transform}>Try Again</button>
                  </section>
                `
              : nothing
          }
          ${
            this.lens.stage === "ready" ||
            (this.lens.stage === "failed" && Boolean(this.lens.input))
              ? html`
                  <div class="overlay-actions">
                    <button class="primary" @click=${this.transform}>Transform with Agent</button>
                  </div>
                `
              : nothing
          }
          ${
            this.lens.stage === "connecting" || this.lens.stage === "transforming"
              ? html`
                  <div class="overlay-actions">
                    <button @click=${this.cancelAgent}>Cancel</button>
                  </div>
                `
              : nothing
          }
          ${
            this.activeLensTab === "translation"
              ? html`
                  <section
                    id="translation-panel"
                    class="lens-panel"
                    role="tabpanel"
                    aria-labelledby="translation-tab"
                    tabindex="0"
                  >
                    ${
                      outputBlocks.length
                        ? html`<div
                            class="lens-content lens-output"
                            data-auto-scroll-container
                            role="document"
                            aria-live="polite"
                          >
                            ${outputBlocks.map((block, index) =>
                              this.renderOutputBlock(block, index === outputBlocks.length - 1),
                            )}
                          </div>`
                        : showsLensProgress(this.lens.stage)
                          ? this.renderLoadingState()
                          : this.renderTranslationEmptyState()
                    }
                  </section>
                `
              : html`
                  <section
                    id="source-panel"
                    class="lens-panel"
                    role="tabpanel"
                    aria-labelledby="source-tab"
                    tabindex="0"
                  >
                    ${
                      sourceJson
                        ? html`<div class="lens-content source-view">
                            ${this.renderInputMediaPreview()}
                            <section class="source-json" aria-labelledby="source-json-heading">
                              <h2 id="source-json-heading">Structured input</h2>
                              <pre
                                class="source-content"
                                aria-label="Normalized Lens source JSON"
                              ><code>${sourceJson}</code></pre>
                            </section>
                          </div>`
                        : html`<p class="empty-state">No normalized source data is available.</p>`
                    }
                    ${context ? this.renderDiagnostics(context) : nothing}
                  </section>
                `
          }
        </main>

        <footer class="overlay-footer">
          <div class="overlay-footer-meta">
            <div class="overlay-footer-status" role="status" title=${STAGE_LABEL[this.lens.stage]}>
              <span class="overlay-stage">${STAGE_LABEL[this.lens.stage]}</span>
              ${
                context
                  ? html`<span class="quality quality-${context.quality}">${context.quality}</span>`
                  : nothing
              }
            </div>
          </div>
          <div class="lens-tabs" role="tablist" aria-label="Lens content">
            ${this.renderLensTab("translation", "Translation")}
            ${this.renderLensTab("source", "Source")}
          </div>
        </footer>
      </div>
    `;
  }

  private renderOutputBlock(block: LensOutputBlock, isLastBlock: boolean) {
    switch (block.type) {
      case "markdown":
        return html`<lens-markdown
          class="markdown-body"
          .state=${
            {
              operationId: this.lens.operation_id,
              markdown: block.text,
              phase: this.lens.stage === "transforming" && isLastBlock ? "streaming" : "settled",
            } satisfies StreamingMarkdownState
          }
          @click=${this.openMarkdownLink}
          @markdown-render-error=${this.handleMarkdownRenderError}
        ></lens-markdown>`;
      case "image": {
        const source = imageDataUrl(block);
        return source
          ? html`<figure class="lens-output-image">
              <img src=${source} alt="Visual output from the agent" />
            </figure>`
          : this.renderUnsupportedOutput(`image (${block.mime_type})`);
      }
      case "unsupported":
        return this.renderUnsupportedOutput(block.content_type);
    }
  }

  private renderInputMediaPreview() {
    const media = this.lens.input?.media ?? [];
    if (!media.length) return nothing;
    const index = Math.min(this.activeInputMediaIndex, media.length - 1);
    const attachment = media[index];
    if (!attachment) return nothing;
    const source = inputMediaPreviewUrl(this.lens, attachment);
    const scopeLabel =
      attachment.scope === "window_fallback" ? "Whole-window fallback" : "AX image region";
    const alt =
      attachment.scope === "window_fallback"
        ? "Whole-window fallback sent to the Agent"
        : `AX image region sent to the Agent for node ${attachment.source_node_id ?? "unknown"}`;
    return html`
      <section class="input-media-preview" aria-labelledby="input-media-heading">
        <header>
          <h2 id="input-media-heading">Input images</h2>
          <span>${index + 1} of ${media.length}</span>
        </header>
        <div class="input-media-carousel" role="group" aria-label="Input image carousel">
          <button
            type="button"
            aria-label="Previous input image"
            ?disabled=${index === 0}
            @click=${() => this.selectInputMedia(index - 1, media.length)}
          >
            <span aria-hidden="true">‹</span>
          </button>
          <ol class="input-media-thumbnails" aria-label="Input image thumbnails">
            ${media.map(
              (candidate, candidateIndex) => html`
                <li>
                  <button
                    type="button"
                    class=${
                      candidateIndex === index
                        ? "input-media-thumbnail is-selected"
                        : "input-media-thumbnail"
                    }
                    aria-label=${`Show input image ${candidateIndex + 1} of ${media.length}`}
                    aria-current=${candidateIndex === index ? "true" : "false"}
                    @click=${() => this.selectInputMedia(candidateIndex, media.length)}
                  >
                    <img
                      src=${inputMediaPreviewUrl(this.lens, candidate) ?? ""}
                      alt=""
                      draggable="false"
                    />
                    <span aria-hidden="true">${candidateIndex + 1}</span>
                  </button>
                </li>
              `,
            )}
          </ol>
          <button
            type="button"
            aria-label="Next input image"
            ?disabled=${index === media.length - 1}
            @click=${() => this.selectInputMedia(index + 1, media.length)}
          >
            <span aria-hidden="true">›</span>
          </button>
        </div>
        <figure>
          ${
            source
              ? html`<img
                  src=${source}
                  alt=${alt}
                  draggable="false"
                  ?hidden=${Boolean(this.inputMediaPreviewError)}
                  @load=${() => {
                    this.inputMediaPreviewError = "";
                  }}
                  @error=${() => {
                    this.inputMediaPreviewError =
                      "The selected input image is no longer available for this operation.";
                  }}
                />`
              : nothing
          }
          ${
            !source || this.inputMediaPreviewError
              ? html`<p class="input-media-error" role="alert">
                  ${
                    this.inputMediaPreviewError ||
                    "The selected input image URI does not match the current operation."
                  }
                </p>`
              : nothing
          }
          <figcaption>
            <dl class="input-media-metadata">
              <div>
                <dt>Scope</dt>
                <dd>${scopeLabel}</dd>
              </div>
              <div>
                <dt>Attachment</dt>
                <dd><code>${attachment.id}</code></dd>
              </div>
              <div>
                <dt>Target</dt>
                <dd><code>${attachment.target_id}</code></dd>
              </div>
              <div>
                <dt>AX node</dt>
                <dd>
                  ${
                    attachment.source_node_id
                      ? html`<code>${attachment.source_node_id}</code>`
                      : "—"
                  }
                </dd>
              </div>
              <div>
                <dt>Pixels</dt>
                <dd>${attachment.pixel_width} × ${attachment.pixel_height}</dd>
              </div>
              <div>
                <dt>Coverage</dt>
                <dd>${attachment.coverage}</dd>
              </div>
              <div>
                <dt>Source bounds</dt>
                <dd>
                  ${attachment.source_bounds.x}, ${attachment.source_bounds.y} ·
                  ${attachment.source_bounds.width} × ${attachment.source_bounds.height}
                </dd>
              </div>
              <div>
                <dt>Captured bounds</dt>
                <dd>
                  ${attachment.captured_bounds.x}, ${attachment.captured_bounds.y} ·
                  ${attachment.captured_bounds.width} × ${attachment.captured_bounds.height}
                </dd>
              </div>
              <div>
                <dt>Coordinates</dt>
                <dd>${attachment.coordinate_space}</dd>
              </div>
              <div>
                <dt>Format</dt>
                <dd>${attachment.mime_type}</dd>
              </div>
              <div>
                <dt>Encoded size</dt>
                <dd>${attachment.encoded_bytes.toLocaleString()} bytes</dd>
              </div>
            </dl>
          </figcaption>
        </figure>
      </section>
    `;
  }

  private selectInputMedia(index: number, total: number): void {
    if (!Number.isInteger(index) || index < 0 || index >= total) return;
    this.activeInputMediaIndex = index;
    this.inputMediaPreviewError = "";
  }

  private renderUnsupportedOutput(contentType: string) {
    return html`<p class="lens-output-unsupported" role="note">
      This agent output type is not supported yet: <code>${contentType}</code>
    </p>`;
  }

  private renderLensTab(tab: LensTab, label: string) {
    const selected = this.activeLensTab === tab;
    return html`
      <button
        type="button"
        id="${tab}-tab"
        class="lens-tab"
        role="tab"
        aria-selected=${selected ? "true" : "false"}
        aria-controls="${tab}-panel"
        tabindex=${selected ? 0 : -1}
        @click=${() => {
          this.activeLensTab = tab;
        }}
        @keydown=${this.handleLensTabKeyDown}
      >
        ${label}
      </button>
    `;
  }

  private renderLoadingState() {
    return html`
      <div class="loading-state" role="status" aria-live="polite">
        <i class="fa-solid fa-spinner fa-spin" aria-hidden="true"></i>
        <strong>${STAGE_LABEL[this.lens.stage]}</strong>
        <p>The source remains available in its own tab while Lens prepares the result.</p>
      </div>
    `;
  }

  private renderTranslationEmptyState() {
    const message = (() => {
      switch (this.lens.stage) {
        case "idle":
          return "Select one or more Lens Targets from the menu bar.";
        case "authentication_required":
          return "Authenticate the selected Agent to continue.";
        case "cancelled":
          return "The Agent transformation was cancelled.";
        case "failed":
          return "The Agent did not produce a translation.";
        case "completed":
          return "The Agent completed without returning displayable content.";
        case "selecting":
        case "extracting":
        case "ready":
        case "connecting":
        case "transforming":
          return "Preparing the Agent translation.";
      }
    })();
    return html`<p class="empty-state">${message}</p>`;
  }

  private applyLensState(next: LensState): void {
    if (next.operation_id !== this.lens.operation_id) {
      this.activeLensTab = "translation";
      this.activeInputMediaIndex = 0;
      this.inputMediaPreviewError = "";
    }
    this.lens = next;
  }

  private handleLensTabKeyDown = (event: KeyboardEvent): void => {
    const nextTab = (() => {
      switch (event.key) {
        case "ArrowLeft":
        case "ArrowRight":
          return this.activeLensTab === "translation" ? "source" : "translation";
        case "Home":
          return "translation";
        case "End":
          return "source";
        default:
          return undefined;
      }
    })();
    if (!nextTab) return;
    event.preventDefault();
    this.activeLensTab = nextTab;
    void this.updateComplete.then(() => {
      this.renderRoot.querySelector<HTMLButtonElement>(`#${nextTab}-tab`)?.focus();
    });
  };

  private openMarkdownLink = async (event: MouseEvent): Promise<void> => {
    const link = event
      .composedPath()
      .find((candidate): candidate is HTMLAnchorElement => candidate instanceof HTMLAnchorElement);
    if (!link) return;
    event.preventDefault();
    const externalUrl = externalMarkdownUrl(link.getAttribute("href") ?? "");
    if (!externalUrl) {
      this.message = "This Markdown link uses an unsupported URL.";
      return;
    }
    try {
      this.message = "";
      await openUrl(externalUrl);
    } catch (error) {
      this.message = `Unable to open Markdown link: ${String(error)}`;
    }
  };

  private handleMarkdownRenderError = (event: CustomEvent<string>): void => {
    this.message = `Unable to render Markdown: ${event.detail}`;
  };

  private renderDiagnostics(context: NonNullable<LensState["context"]>) {
    const mediaMetrics = [
      [
        "AX image regions",
        context.media.filter((item) => item.scope === "ax_element_region").length,
      ],
      ["Window fallbacks", context.media.filter((item) => item.scope === "window_fallback").length],
      ["PNG bytes", context.media.reduce((total, item) => total + item.encoded_bytes, 0)],
      [
        "Omitted images",
        context.media_omissions.reduce((total, item) => total + item.omitted_count, 0),
      ],
    ] as const;
    const agentMetrics = this.lens.agent
      ? ([
          ["ACP Agent", `${this.lens.agent.adapter_name} ${this.lens.agent.adapter_version}`],
          ["Session updates", this.lens.agent.received_updates],
          ["Stop reason", this.lens.agent.stop_reason ?? "—"],
        ] as const)
      : [];
    const diagnosticCount = context.diagnostics.length;

    return html`
      <details class="extraction-diagnostics">
        <summary>
          Diagnostics
          <span class="diagnostic-count"
            >${
              diagnosticCount === 0
                ? "No messages"
                : `${diagnosticCount} ${diagnosticCount === 1 ? "message" : "messages"}`
            }</span
          >
        </summary>
        <div class="diagnostics-layout">
          ${context.sources.map((source, index) => {
            const accessibility = source.capture;
            const metrics = [
              ["Quality", source.quality],
              ["Visited nodes", accessibility.metrics.visited_nodes],
              ["UTF-8 bytes", accessibility.metrics.text_bytes],
              ["Off-window text nodes", accessibility.metrics.offscreen_text_nodes],
              ["Virtualization signals", accessibility.metrics.virtualization_signals],
              ["Child read errors", accessibility.metrics.children_read_errors],
              ["URI resource references", accessibility.metrics.resource_ref_count],
              ["URI UTF-8 bytes", accessibility.metrics.resource_uri_bytes],
              ["Omitted URI references", accessibility.metrics.omitted_resource_refs],
              ["URI read errors", accessibility.metrics.resource_read_errors],
              ["Nodes truncated", accessibility.metrics.truncated_nodes ? "Yes" : "No"],
              ["Text truncated", accessibility.metrics.truncated_text ? "Yes" : "No"],
            ] as const;
            const headingId = `accessibility-metrics-heading-${index}`;
            const sourceLabel = source.source.window_title
              ? `${source.source.application} — ${source.source.window_title}`
              : source.source.application;
            return html`
              <section class="diagnostic-group" aria-labelledby=${headingId}>
                <h2 id=${headingId}>Accessibility — ${sourceLabel}</h2>
                <dl class="metrics">
                  ${metrics.map(
                    ([label, value]) => html`
                      <div>
                        <dt>${label}</dt>
                        <dd>${value}</dd>
                      </div>
                    `,
                  )}
                </dl>
              </section>
            `;
          })}
          <section class="diagnostic-group" aria-labelledby="media-metrics-heading">
            <h2 id="media-metrics-heading">AX-linked images</h2>
            <dl class="metrics">
              ${mediaMetrics.map(
                ([label, value]) => html`
                  <div>
                    <dt>${label}</dt>
                    <dd>${value}</dd>
                  </div>
                `,
              )}
            </dl>
          </section>
          ${
            agentMetrics.length
              ? html`
                  <section class="diagnostic-group" aria-labelledby="agent-metrics-heading">
                    <h2 id="agent-metrics-heading">Agent session</h2>
                    <dl class="metrics">
                      ${agentMetrics.map(
                        ([label, value]) => html`
                          <div>
                            <dt>${label}</dt>
                            <dd>${value}</dd>
                          </div>
                        `,
                      )}
                    </dl>
                  </section>
                `
              : nothing
          }
          <section
            class="diagnostic-group diagnostic-messages"
            aria-labelledby="diagnostic-messages-heading"
          >
            <h2 id="diagnostic-messages-heading">Messages</h2>
            ${
              diagnosticCount
                ? html`<ul>
                    ${context.diagnostics.map((item) => html`<li>${item}</li>`)}
                  </ul>`
                : html`<p>No extraction warnings or errors.</p>`
            }
          </section>
        </div>
      </details>
    `;
  }

  private async setAgent(agent: AgentKind): Promise<void> {
    this.busy = true;
    try {
      await invoke<AgentSelectionState>("set_agent", { agent });
      this.message = "";
    } catch (error) {
      this.message = String(error);
    } finally {
      this.busy = false;
    }
  }

  private async authenticateAgentSelection(methodId: string): Promise<void> {
    this.busy = true;
    this.message = "Starting Agent authentication.";
    try {
      await invoke<AgentSelectionState>("authenticate_agent_selection", { methodId });
      this.message = "";
    } catch (error) {
      this.message = String(error);
    } finally {
      this.busy = false;
    }
  }

  private reauthenticateAgentSelection = async (): Promise<void> => {
    const agent = selectedAgent(this.agentSelection);
    if (!agent) return;
    const label = agent === "claude" ? "Claude" : "Codex";
    const approved = await confirm(`Reauthentication signs out of ${label} first. Continue?`, {
      title: `Reauthenticate ${label}`,
      kind: "warning",
    });
    if (!approved) return;

    this.busy = true;
    this.message = `Preparing to reauthenticate ${label}.`;
    try {
      await invoke<AgentSelectionState>("reauthenticate_agent_selection");
      this.message = "";
    } catch (error) {
      this.message = String(error);
    } finally {
      this.busy = false;
    }
  };

  private signOutAgentSelection = async (): Promise<void> => {
    const agent = selectedAgent(this.agentSelection);
    if (!agent) return;
    const label = agent === "claude" ? "Claude" : "Codex";
    const approved = await confirm(
      `Sign out of ${label}? This changes the authentication used by its existing CLI.`,
      { title: `Sign Out of ${label}`, kind: "warning" },
    );
    if (!approved) return;

    this.busy = true;
    this.message = `Signing out of ${label}.`;
    try {
      await invoke<AgentSelectionState>("sign_out_agent_selection");
      this.message = "";
    } catch (error) {
      this.message = String(error);
    } finally {
      this.busy = false;
    }
  };

  private chooseDirectory = async (): Promise<void> => {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: this.config?.working_directory,
      title: "Choose Working Directory",
    });
    if (typeof selected !== "string") return;
    try {
      await invoke<AppConfig>("set_working_directory", { path: selected });
      this.message = "Working directory updated.";
    } catch (error) {
      this.message = String(error);
    }
  };

  private editResponsePrompt = (event: Event): void => {
    this.responsePromptDraft = (event.currentTarget as HTMLTextAreaElement).value;
    this.responsePromptDirty = this.responsePromptDraft !== this.config?.response_prompt;
  };

  private saveResponsePrompt = async (event: SubmitEvent): Promise<void> => {
    event.preventDefault();
    if (!this.responsePromptDirty || !this.responsePromptDraft.trim()) return;
    this.busy = true;
    try {
      const config = await invoke<AppConfig>("set_response_prompt", {
        responsePrompt: this.responsePromptDraft,
      });
      this.config = config;
      this.responsePromptDraft = config.response_prompt;
      this.responsePromptDirty = false;
      this.message = "Agent prompt updated.";
    } catch (error) {
      this.message = String(error);
    } finally {
      this.busy = false;
    }
  };

  private resetResponsePrompt = async (): Promise<void> => {
    const approved = await confirm("Reset the Agent Prompt to the built-in default?", {
      title: "Reset Agent Prompt",
      kind: "warning",
    });
    if (!approved) return;
    this.busy = true;
    try {
      const config = await invoke<AppConfig>("reset_response_prompt");
      this.config = config;
      this.responsePromptDraft = config.response_prompt;
      this.responsePromptDirty = false;
      this.message = "Agent prompt reset to the built-in default.";
    } catch (error) {
      this.message = String(error);
    } finally {
      this.busy = false;
    }
  };

  private requestPermission = async (): Promise<void> => {
    await invoke<boolean>("request_accessibility_permission");
    this.message = "Allow Lens in System Settings, then check again.";
    window.setTimeout(() => void this.refreshPermission(), 1_200);
  };

  private async refreshPermission(): Promise<void> {
    if (document.visibilityState !== "visible" || this.trusted) return;
    try {
      this.trusted = await invoke<boolean>("accessibility_permission");
      if (this.trusted) this.message = "Accessibility permission confirmed.";
    } catch {
      // A transient IPC failure must not replace a more useful user-facing message.
    }
  }

  private addSelectionTarget = async (): Promise<void> => {
    const operationId = this.lens.operation_id;
    if (!operationId) return;
    this.busy = true;
    this.message = "";
    try {
      await invoke<LensState>("add_lens_target", { operationId });
    } catch (error) {
      this.message = String(error);
    } finally {
      this.busy = false;
    }
  };

  private async removeSelectionTarget(targetId: string): Promise<void> {
    const operationId = this.lens.operation_id;
    if (!operationId) return;
    this.busy = true;
    this.message = "";
    try {
      await invoke<LensState>("remove_lens_target", { operationId, targetId });
    } catch (error) {
      this.message = String(error);
    } finally {
      this.busy = false;
    }
  }

  private confirmSelectionTargets = async (): Promise<void> => {
    const operationId = this.lens.operation_id;
    if (!operationId) return;
    this.busy = true;
    this.message = "";
    try {
      await invoke<LensState>("confirm_lens_targets", { operationId });
    } catch (error) {
      this.message = String(error);
      this.busy = false;
    }
  };

  private transform = async (): Promise<void> => {
    const operationId = this.lens.operation_id;
    if (!operationId) return;
    this.message = "Connecting to the agent.";
    try {
      await invoke<LensState>("transform_lens", { operationId });
      this.message = "";
    } catch (error) {
      this.message = String(error);
    }
  };

  private async authenticate(methodId: string): Promise<void> {
    const operationId = this.lens.operation_id;
    if (!operationId) return;
    this.message = "Starting agent authentication.";
    try {
      await invoke<LensState>("authenticate_agent", { operationId, methodId });
      this.message = "";
    } catch (error) {
      this.message = String(error);
    }
  }

  private cancelAgent = async (): Promise<void> => {
    const operationId = this.lens.operation_id;
    const runId = this.lens.agent?.run_id;
    if (!operationId || !runId) return;
    try {
      await invoke<LensState>("cancel_agent", { operationId, runId });
      this.message = "";
    } catch (error) {
      this.message = String(error);
    }
  };

  private closeWindow = (): void => {
    void getCurrentWindow().close();
  };
}
