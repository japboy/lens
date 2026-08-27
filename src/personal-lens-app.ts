import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import fontAwesomeStyles from "@fortawesome/fontawesome-free/css/fontawesome.css?inline";
import fontAwesomeSolidStyles from "@fortawesome/fontawesome-free/css/solid.css?inline";
import { LitElement, css, html, nothing, unsafeCSS } from "lit";
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

export class PersonalLensApp extends LitElement {
  static properties = {
    config: { state: true },
    agentSelection: { state: true },
    agentRuntime: { state: true },
    lens: { state: true },
    trusted: { state: true },
    busy: { state: true },
    message: { state: true },
    activeLensTab: { state: true },
    responsePromptDraft: { state: true },
    responsePromptDirty: { state: true },
  };

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

  declare private config: AppConfig | undefined;
  declare private agentSelection: AgentSelectionState;
  declare private agentRuntime: AgentRuntimeState;
  declare private lens: LensState;
  declare private trusted: boolean;
  declare private busy: boolean;
  declare private message: string;
  declare private activeLensTab: LensTab;
  declare private responsePromptDraft: string;
  declare private responsePromptDirty: boolean;
  private unlisten: UnlistenFn[];
  private permissionTimer?: number;
  private revision: number;
  private loadGeneration: number;

  constructor() {
    super();
    this.config = undefined;
    this.agentSelection = { stage: "unselected", auth_methods: [] };
    this.agentRuntime = {
      stage: "not_installed",
      downloaded_bytes: 0,
    };
    this.lens = { stage: "idle", output_blocks: [] };
    this.trusted = false;
    this.busy = false;
    this.message = "";
    this.activeLensTab = "translation";
    this.responsePromptDraft = "";
    this.responsePromptDirty = false;
    this.unlisten = [];
    this.revision = -1;
    this.loadGeneration = 0;
  }

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

  private get view(): "settings" | "overlay" {
    return new URLSearchParams(window.location.search).get("view") === "overlay"
      ? "overlay"
      : "settings";
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
    return this.view === "overlay" ? this.renderOverlay() : this.renderSettings();
  }

  private renderSettings() {
    return html`
      <main class="settings-shell">
        <header>
          <h1>PersonalLens</h1>
          <p>Transform accessibility content from the selected window with the selected agent.</p>
        </header>

        <section aria-labelledby="agent-heading">
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
          <p class="help">The ACP agent, not PersonalLens, manages authentication credentials.</p>
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

        <section aria-labelledby="prompt-heading">
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
              Controls how the Agent transforms the source. PersonalLens appends fixed source-data
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

        <section aria-labelledby="cwd-heading">
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

        <section aria-labelledby="permission-heading">
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

  private renderOverlay() {
    const extraction = this.lens.extraction;
    const target = this.lens.target;
    const outputBlocks = lensOutputBlocks(this.lens);
    const sourceJson = lensSourceJson(this.lens);
    const activeAgent = this.lens.agent;
    const authenticationMethods = supportedAuthMethods(this.lens);
    const applicationName = target?.application_name ?? "PersonalLens";
    const windowContext = target?.title ? `${applicationName} — ${target.title}` : applicationName;
    return html`
      <div class="overlay-shell">
        <header class="overlay-header" data-tauri-drag-region="deep">
          <h1 class="overlay-title" title=${windowContext}>
            <strong>${applicationName}</strong>
            ${target?.title ? html`<span> — ${target.title}</span>` : nothing}
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
                        ? html`<pre
                            class="lens-content source-content"
                            aria-label="Normalized Lens source JSON"
                          ><code>${sourceJson}</code></pre>`
                        : html`<p class="empty-state">No normalized source data is available.</p>`
                    }
                    ${extraction ? this.renderDiagnostics(extraction) : nothing}
                  </section>
                `
          }
        </main>

        <footer class="overlay-footer">
          <div class="overlay-footer-meta">
            <div class="overlay-footer-status" role="status" title=${STAGE_LABEL[this.lens.stage]}>
              <span class="overlay-stage">${STAGE_LABEL[this.lens.stage]}</span>
              ${
                extraction
                  ? html`<span class="quality quality-${extraction.quality}"
                      >${extraction.quality}</span
                    >`
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
        return html`<personal-lens-markdown
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
        ></personal-lens-markdown>`;
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
        <p>The source remains available in its own tab while PersonalLens prepares the result.</p>
      </div>
    `;
  }

  private renderTranslationEmptyState() {
    const message = (() => {
      switch (this.lens.stage) {
        case "idle":
          return "Select a Lens Target from the menu bar.";
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

  private renderDiagnostics(extraction: NonNullable<LensState["extraction"]>) {
    const extractionMetrics = [
      ["Visited nodes", extraction.metrics.visited_nodes],
      ["UTF-8 bytes", extraction.metrics.text_bytes],
      ["Off-window text nodes", extraction.metrics.offscreen_text_nodes],
      ["Virtualization signals", extraction.metrics.virtualization_signals],
      ["Child read errors", extraction.metrics.children_read_errors],
      ["Nodes truncated", extraction.metrics.truncated_nodes ? "Yes" : "No"],
      ["Text truncated", extraction.metrics.truncated_text ? "Yes" : "No"],
    ] as const;
    const agentMetrics = this.lens.agent
      ? ([
          ["ACP Agent", `${this.lens.agent.adapter_name} ${this.lens.agent.adapter_version}`],
          ["Session updates", this.lens.agent.received_updates],
          ["Stop reason", this.lens.agent.stop_reason ?? "—"],
        ] as const)
      : [];
    const diagnosticCount = extraction.diagnostics.length;

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
          <section class="diagnostic-group" aria-labelledby="extraction-metrics-heading">
            <h2 id="extraction-metrics-heading">Extraction</h2>
            <dl class="metrics">
              ${extractionMetrics.map(
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
                    ${extraction.diagnostics.map((item) => html`<li>${item}</li>`)}
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
    this.message = "Allow PersonalLens in System Settings, then check again.";
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

customElements.define("personal-lens-app", PersonalLensApp);
