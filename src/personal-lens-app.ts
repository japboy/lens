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
  AgentSelectionState,
  AppConfig,
  AppSnapshot,
  LensState,
} from "./types";
import {
  AGENT_SELECTION_LABEL,
  lensTranslationText,
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
    lens: { state: true },
    trusted: { state: true },
    busy: { state: true },
    message: { state: true },
    activeLensTab: { state: true },
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
  declare private lens: LensState;
  declare private trusted: boolean;
  declare private busy: boolean;
  declare private message: string;
  declare private activeLensTab: LensTab;
  private unlisten: UnlistenFn[];
  private permissionTimer?: number;
  private revision: number;
  private loadGeneration: number;

  constructor() {
    super();
    this.config = undefined;
    this.agentSelection = { stage: "unselected", auth_methods: [] };
    this.lens = { stage: "idle" };
    this.trusted = false;
    this.busy = false;
    this.message = "";
    this.activeLensTab = "translation";
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
    this.config = next.config;
    this.agentSelection = next.agent_selection;
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
            ?disabled=${this.busy || !this.config || this.agentSelection.stage === "signing_out"}
          >
            <legend class="visually-hidden">AI agent to use</legend>
            ${this.agentOption("claude", "Claude")}
            ${this.agentOption("codex", "Codex")}
          </fieldset>
          <p class="help">The ACP agent, not PersonalLens, manages authentication credentials.</p>
          <output class=${this.agentSelection.stage === "selected" ? "status-ok" : "status-warning"}>
            ${this.agentSelection.message ??
            this.agentSelection.error ??
            AGENT_SELECTION_LABEL[this.agentSelection.stage]}
          </output>
          ${this.renderAgentSelectionAuthentication()}
          ${this.renderSelectedAgentActions()}
        </section>

        <section aria-labelledby="cwd-heading">
          <h2 id="cwd-heading">Working Directory</h2>
          <div class="directory-row">
            <input
              aria-label="Working Directory"
              readonly
              .value=${this.config?.working_directory ?? ""}
            />
            <button @click=${this.chooseDirectory} ?disabled=${this.busy}>Choose…</button>
          </div>
          <p class="help">The default is your home directory. The agent uses this directory as the cwd for resolving its own project instructions and memory.</p>
        </section>

        <section aria-labelledby="permission-heading">
          <h2 id="permission-heading">Accessibility</h2>
          <div class="permission-row">
            <output class=${this.trusted ? "status-ok" : "status-warning"}>
              ${this.trusted ? "Allowed" : "Permission required"}
            </output>
            ${this.trusted
              ? nothing
              : html`<button @click=${this.requestPermission}>Open System Settings</button>`}
          </div>
        </section>

        <footer>
          <span role="status">${this.message || STAGE_LABEL[this.lens.stage]}</span>
        </footer>
      </main>
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
        ${methods.length
          ? methods.map(
              (method) => html`
                <button @click=${() => this.authenticateAgentSelection(method.id)}>
                  Authenticate with ${method.name}…
                </button>
              `,
            )
          : html`<p>Authenticate with this Agent's existing CLI, then select it again.</p>`}
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
    const translationText = lensTranslationText(this.lens);
    const sourceText = this.lens.input?.text ?? extraction?.text ?? "";
    const activeAgent = this.lens.agent;
    const authenticationMethods = supportedAuthMethods(this.lens);
    return html`
      <main class="overlay-shell">
        <header class="overlay-titlebar">
          <div>
            <strong>${target?.application_name ?? "PersonalLens"}</strong>
            ${target?.title ? html`<span> — ${target.title}</span>` : nothing}
          </div>
          <button class="close-button" aria-label="Close Lens" @click=${this.closeWindow}>×</button>
        </header>

        <div class="overlay-status" role="status">
          <span>${STAGE_LABEL[this.lens.stage]}</span>
          ${extraction
            ? html`<span class="quality quality-${extraction.quality}">${extraction.quality}</span>`
            : nothing}
        </div>

        ${this.message
          ? html`<p class="error" role="alert">${this.message}</p>`
          : nothing}

        ${this.lens.error
          ? html`<p class="error" role="alert">${this.lens.error}</p>`
          : nothing}

        ${activeAgent?.authentication_message
          ? html`<p class="notice" role="status">${activeAgent.authentication_message}</p>`
          : nothing}

        ${this.lens.stage === "authentication_required"
          ? html`
              <section class="overlay-actions" aria-label="Agent authentication">
                ${authenticationMethods.length
                  ? authenticationMethods.map(
                      (method) => html`
                        <button @click=${() => this.authenticate(method.id)}>
                          Authenticate with ${method.name}…
                        </button>
                      `,
                    )
                  : html`<p>Authenticate with this agent's existing CLI, then try again.</p>`}
                <button @click=${this.transform}>Try Again</button>
              </section>
            `
          : nothing}

        ${this.lens.stage === "ready" ||
        (this.lens.stage === "failed" && Boolean(this.lens.input))
          ? html`
              <div class="overlay-actions">
                <button class="primary" @click=${this.transform}>Transform with Agent</button>
              </div>
            `
          : nothing}

        ${this.lens.stage === "connecting" || this.lens.stage === "transforming"
          ? html`
              <div class="overlay-actions">
                <button @click=${this.cancelAgent}>Cancel</button>
              </div>
            `
          : nothing}

        <div class="lens-tabs" role="tablist" aria-label="Lens content">
          ${this.renderLensTab("translation", "Translation")}
          ${this.renderLensTab("source", "Source")}
        </div>

        ${this.activeLensTab === "translation"
          ? html`
              <section
                id="translation-panel"
                class="lens-panel"
                role="tabpanel"
                aria-labelledby="translation-tab"
              >
                ${translationText
                  ? html`<personal-lens-markdown
                      class="lens-content markdown-body"
                      role="document"
                      aria-live="polite"
                      .state=${{
                        operationId: this.lens.operation_id,
                        markdown: translationText,
                        phase: this.lens.stage === "transforming" ? "streaming" : "settled",
                      } satisfies StreamingMarkdownState}
                      @click=${this.openMarkdownLink}
                      @markdown-render-error=${this.handleMarkdownRenderError}
                    ></personal-lens-markdown>`
                  : showsLensProgress(this.lens.stage)
                    ? this.renderLoadingState()
                    : this.renderTranslationEmptyState()}
              </section>
            `
          : html`
              <section
                id="source-panel"
                class="lens-panel"
                role="tabpanel"
                aria-labelledby="source-tab"
              >
                ${sourceText
                  ? html`<article class="lens-content source-content">${sourceText}</article>`
                  : html`<p class="empty-state">No source text is available.</p>`}
                ${extraction ? this.renderDiagnostics(extraction) : nothing}
              </section>
            `}
      </main>
    `;
  }

  private renderLensTab(tab: LensTab, label: string) {
    const selected = this.activeLensTab === tab;
    return html`
      <button
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
          return "The Agent completed without returning Markdown content.";
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
    this.message = `Unable to render the streaming Markdown update: ${event.detail}`;
  };

  private renderDiagnostics(extraction: NonNullable<LensState["extraction"]>) {
    return html`
      <details class="extraction-diagnostics">
        <summary>Extraction diagnostics</summary>
        <dl class="metrics">
          <dt>Nodes</dt><dd>${extraction.metrics.visited_nodes}</dd>
          <dt>UTF-8 bytes</dt><dd>${extraction.metrics.text_bytes}</dd>
          <dt>Off-window text nodes</dt><dd>${extraction.metrics.offscreen_text_nodes}</dd>
          <dt>Virtualization signals</dt><dd>${extraction.metrics.virtualization_signals}</dd>
          ${this.lens.agent
            ? html`
                <dt>ACP Agent</dt>
                <dd>${this.lens.agent.adapter_name} ${this.lens.agent.adapter_version}</dd>
                <dt>Session updates</dt><dd>${this.lens.agent.received_updates}</dd>
                <dt>Stop reason</dt><dd>${this.lens.agent.stop_reason ?? "—"}</dd>
              `
            : nothing}
        </dl>
        ${extraction.diagnostics.length
          ? html`<ul>${extraction.diagnostics.map((item) => html`<li>${item}</li>`)}</ul>`
          : html`<p>No diagnostics.</p>`}
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
      await invoke<AgentSelectionState>(
        "authenticate_agent_selection",
        { methodId },
      );
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
    const approved = await confirm(
      `Reauthentication signs out of ${label} first. Continue?`,
      { title: `Reauthenticate ${label}`, kind: "warning" },
    );
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
