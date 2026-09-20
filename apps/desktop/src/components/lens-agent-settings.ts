import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type {
  AgentKind,
  ExternalAgentProfile,
  AgentRuntimeState,
  AgentSelectionState,
} from "../types";
import {
  AGENT_RUNTIME_LABEL,
  agentLabel,
  sameAgent,
  AGENT_SELECTION_LABEL,
  isAgentRuntimeActive,
  selectedAgent,
} from "../view-model";
import { formatExternalAgentCommand, parseExternalAgentCommand } from "../external-agent-command";
import { AGENT_INTENT_EVENT, dispatchComponentEvent, type AgentIntent } from "./events";

@customElement("lens-agent-settings")
export class LensAgentSettings extends LitElement {
  @property({ attribute: false })
  selection: AgentSelectionState | undefined;

  @property({ attribute: false })
  runtime: AgentRuntimeState | undefined;

  @property({ type: Boolean })
  disabled = false;

  @property({ attribute: false }) profiles: ExternalAgentProfile[] = [];
  @state() private managedSelection: "claude" | "codex" = "claude";
  @state() private editingId: string | undefined;
  @state() private commandDrafts: Record<string, string> = {};
  @state() private drafts: Record<string, ExternalAgentProfile> = {};
  private draftRevision = 0;
  private browseSavedProfile = "";

  protected willUpdate(changed: PropertyValues<this>): void {
    if (
      changed.has("selection") &&
      !sameAgent(changed.get("selection")?.candidate, this.selection?.candidate)
    ) {
      this.draftRevision += 1;
      if (typeof this.selection?.candidate === "string")
        this.managedSelection = this.selection.candidate;
      this.editingId =
        typeof this.selection?.candidate === "object"
          ? this.selection.candidate.external
          : undefined;
    }
    if (changed.has("profiles")) {
      const before = changed.get("profiles") ?? [];
      for (const profile of this.profiles) {
        const old = before.find((p) => p.id === profile.id);
        if (!this.drafts[profile.id] || JSON.stringify(old) !== JSON.stringify(profile)) {
          this.drafts = { ...this.drafts, [profile.id]: structuredClone(profile) };
          this.commandDrafts = {
            ...this.commandDrafts,
            [profile.id]: formatExternalAgentCommand(profile),
          };
          this.draftRevision += 1;
        }
      }
      for (const old of before) {
        if (!this.profiles.some((p) => p.id === old.id)) {
          const { [old.id]: _removed, ...remaining } = this.drafts;
          this.drafts = remaining;
          if (this.editingId === old.id) this.editingId = undefined;
          this.draftRevision += 1;
        }
      }
    }
  }
  acceptResetPresets(profiles: ExternalAgentProfile[], agent: AgentKind): void {
    this.draftRevision += 1;
    this.browseSavedProfile = "";
    this.profiles = structuredClone(profiles);
    this.drafts = Object.fromEntries(
      profiles.map((profile) => [profile.id, structuredClone(profile)]),
    );
    this.commandDrafts = Object.fromEntries(
      profiles.map((profile) => [profile.id, formatExternalAgentCommand(profile)]),
    );
    this.editingId = typeof agent === "object" ? agent.external : undefined;
    if (typeof agent === "string") this.managedSelection = agent;
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.draftRevision += 1;
  }
  private savedFingerprint(): string {
    return JSON.stringify(this.profiles.find((p) => p.id === this.editingId));
  }
  acceptExternalExecutable(path: string, draftRevision: number): boolean {
    if (
      !this.isConnected ||
      !this.editingId ||
      draftRevision !== this.draftRevision ||
      this.savedFingerprint() !== this.browseSavedProfile
    )
      return false;
    const draft = this.drafts[this.editingId]!;
    let args = draft.args;
    try {
      const line = this.commandDrafts[this.editingId] ?? "";
      args = line.trim() ? parseExternalAgentCommand(line).args : draft.args;
    } catch {
      return false;
    }
    this.editCommand(formatExternalAgentCommand({ command: path, args }));
    return true;
  }
  private editDraft(change: Partial<ExternalAgentProfile>): void {
    if (!this.editingId) return;
    this.drafts = {
      ...this.drafts,
      [this.editingId]: { ...this.drafts[this.editingId], ...change },
    };
    this.draftRevision += 1;
  }
  private addProfile(): void {
    const id = crypto.randomUUID();
    this.drafts = { ...this.drafts, [id]: { id, name: "", command: "", args: [] } };
    this.commandDrafts = { ...this.commandDrafts, [id]: "" };
    this.editingId = id;
    this.draftRevision += 1;
  }

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    if (!this.selection || !this.runtime) return nothing;
    const controlsDisabled =
      this.disabled ||
      isAgentRuntimeActive(this.runtime.stage) ||
      ["checking", "authenticating", "signing_out"].includes(this.selection.stage);
    const methods =
      this.selection.stage === "authentication_required"
        ? this.selection.auth_methods.filter((method) => method.supported)
        : [];
    const selected = selectedAgent(this.selection);
    const selectedLabel = selected ? agentLabel(selected, this.profiles) : undefined;
    const externalSelected = typeof this.selection.candidate === "object";
    const draft = this.editingId ? this.drafts[this.editingId] : undefined;
    const saved = this.profiles.find((p) => p.id === this.editingId);
    const total = this.runtime.total_bytes;

    return html`
      <fieldset class="external-executable-settings" ?disabled=${controlsDisabled}>
        <legend class="visually-hidden">AI agent to use</legend>
        <label class="settings-field"
          ><span>Agent</span
          ><select
            aria-label="Agent"
            @change=${(event: Event) => {
              const value = (event.target as HTMLSelectElement).value;
              this.draftRevision += 1;
              this.editingId = value === "claude" || value === "codex" ? undefined : value;
              if (!this.editingId) this.managedSelection = value as "claude" | "codex";
              if (!this.editingId)
                this.emit({ type: "select", agent: value as "claude" | "codex" });
              else if (this.profiles.some((profile) => profile.id === value))
                this.emit({ type: "select", agent: { external: value } });
            }}
          >
            <option
              value="claude"
              .selected=${!this.editingId && this.managedSelection === "claude"}
            >
              Claude
            </option>
            <option value="codex" .selected=${!this.editingId && this.managedSelection === "codex"}>
              Codex
            </option>
            ${Object.values(this.drafts).map((profile) => html`<option value=${profile.id} .selected=${profile.id === this.editingId}>${profile.name || "New preset"}${this.profiles.some((saved) => saved.id === profile.id) ? (Object.values(this.drafts).filter((other) => other.name === profile.name).length > 1 ? ` — ${profile.command}` : "") : " (unsaved)"}</option>`)}
          </select></label
        >
        <div class="agent-actions">
          <button
            ?disabled=${Object.keys(this.drafts).length >= 16}
            @click=${() => this.addProfile()}
          >
            Add preset
          </button>
          <button @click=${() => this.emit({ type: "reset-agent-presets" })}>
            Reset Agent Presets…
          </button>
        </div>
        ${
          draft
            ? html`
                <label class="settings-field"
                  ><span>Display name</span
                  ><input
                    class="external-executable-field"
                    aria-label="Connection name"
                    .value=${draft.name}
                    @input=${(event: Event) => this.editDraft({ name: (event.target as HTMLInputElement).value })}
                /></label>
                <label class="settings-field"
                  ><span>Command</span
                  ><input
                    class="external-executable-field"
                    aria-label="ACP command"
                    spellcheck="false"
                    autocomplete="off"
                    placeholder="goose acp"
                    .value=${this.commandDrafts[draft.id] ?? ""}
                    @input=${(event: Event) => this.editCommand((event.target as HTMLInputElement).value)}
                /></label>
                <p class="help">
                  Enter a single-line command using an executable name from PATH or an absolute
                  path, such as goose acp. Quote paths or arguments containing spaces. Quotes and
                  backslash escapes group literal arguments; shell expansion is not supported.
                </p>
                ${this.commandError() ? html`<p role="alert">${this.commandError()}</p>` : nothing}
                ${JSON.stringify(draft) !== JSON.stringify(saved) || Boolean(this.commandError()) ? html`<p class="help" role="status">Unsaved connection changes have not been verified.</p>` : nothing}
                <div class="agent-actions">
                  <button
                    @click=${() => {
                      this.draftRevision += 1;
                      this.browseSavedProfile = this.savedFingerprint();
                      this.emit({
                        type: "choose-external-executable",
                        draftRevision: this.draftRevision,
                        defaultPath: this.drafts[this.editingId!]?.command || undefined,
                      });
                    }}
                  >
                    Browse…
                  </button>
                  <button
                    ?disabled=${!draft.name.trim() || Boolean(this.commandError())}
                    @click=${() => this.emit({ type: "save-external-agent", profile: { id: this.editingId!, name: this.drafts[this.editingId!]!.name, command_line: this.commandDrafts[this.editingId!]! } })}
                  >
                    Save and Verify
                  </button>
                  ${
                    saved
                      ? html`<button
                          @click=${() => this.emit({ type: "delete-external-agent", id: draft.id })}
                        >
                          Delete preset
                        </button>`
                      : html`<button
                          @click=${() => {
                            const { [draft.id]: _removed, ...remaining } = this.drafts;
                            this.drafts = remaining;
                            const candidate = this.selection?.candidate;
                            this.editingId =
                              typeof candidate === "object" ? candidate.external : undefined;
                            if (typeof candidate === "string") this.managedSelection = candidate;
                            this.draftRevision += 1;
                          }}
                        >
                          Discard draft
                        </button>`
                  }
                </div>
              `
            : nothing
        }
        <p class="help">
          Install, update and configure external agents with their own CLI. Editing and browsing do
          not start a process. Save and Verify saves the connection and starts its ACP executable to
          verify it.
        </p>
      </fieldset>
      <p class="help">The ACP agent, not Lens, manages authentication credentials.</p>
      <div class="runtime-status" aria-live="polite">
        <output class=${this.runtime.stage === "failed" ? "status-warning" : "runtime-message"}>
          ${this.runtime.error ?? this.runtime.message ?? AGENT_RUNTIME_LABEL[this.runtime.stage]}
        </output>
        ${
          this.runtime.stage === "downloading"
            ? total === undefined
              ? html`<progress aria-label="Agent runtime download progress"></progress>`
              : html`<progress
                  aria-label="Agent runtime download progress"
                  .value=${this.runtime.downloaded_bytes}
                  max=${total}
                ></progress>`
            : nothing
        }
      </div>
      <output class=${this.selection.stage === "selected" ? "status-ok" : "status-warning"}>
        ${
          this.selection.error ??
          this.selection.message ??
          AGENT_SELECTION_LABEL[this.selection.stage]
        }
      </output>
      ${
        this.selection.stage === "authentication_required" && !externalSelected
          ? html`<div class="agent-actions" aria-label="Agent authentication">
              ${
                methods.length
                  ? methods.map(
                      (method) => html`
                        <button
                          ?disabled=${this.disabled}
                          @click=${() => this.emit({ type: "authenticate", methodId: method.id })}
                        >
                          Authenticate with ${method.name}…
                        </button>
                      `,
                    )
                  : html`<p>Authenticate with this Agent's existing CLI, then select it again.</p>`
              }
            </div>`
          : nothing
      }
      ${
        !this.editingId &&
        ["unselected", "failed", "authentication_required", "history_selected"].includes(
          this.selection.stage,
        )
          ? html`<div class="agent-actions">
              <button
                ?disabled=${controlsDisabled}
                @click=${() => this.emit({ type: "select", agent: this.managedSelection })}
              >
                Verify connection
              </button>
            </div>`
          : nothing
      }
      ${
        selected && this.selection.supports_logout
          ? html`<div class="agent-actions" aria-label="${selectedLabel} authentication management">
              <button
                ?disabled=${this.disabled}
                @click=${() => this.emit({ type: "reauthenticate" })}
              >
                Reauthenticate…
              </button>
              <button ?disabled=${this.disabled} @click=${() => this.emit({ type: "sign-out" })}>
                Sign Out…
              </button>
            </div>`
          : nothing
      }
    `;
  }

  private editCommand(value: string): void {
    if (!this.editingId) return;
    this.commandDrafts = { ...this.commandDrafts, [this.editingId]: value };
    this.draftRevision += 1;
    try {
      this.editDraft(parseExternalAgentCommand(value));
    } catch {
      /* Keep the incomplete text as a local draft. */
    }
  }
  private commandError(): string | undefined {
    if (!this.editingId) return undefined;
    try {
      parseExternalAgentCommand(this.commandDrafts[this.editingId] ?? "");
      return undefined;
    } catch (error) {
      return (error as Error).message;
    }
  }

  private emit(intent: AgentIntent): void {
    dispatchComponentEvent(this, AGENT_INTENT_EVENT, intent);
  }
}
