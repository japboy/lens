import { agentIcon } from "../agent-icons";
import "./lens-select";
import type { LensSelect } from "./lens-select";
import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type {
  AgentKind,
  ManagedAgentKind,
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
import {
  formatExternalAgentArguments,
  parseExternalAgentArguments,
} from "../external-agent-command";
import { AGENT_INTENT_EVENT, dispatchComponentEvent, type AgentIntent } from "./events";

@customElement("lens-agent-settings")
export class LensAgentSettings extends LitElement {
  @property({ attribute: false })
  selection: AgentSelectionState | undefined;

  @property({ attribute: false })
  runtime: AgentRuntimeState | undefined;

  @property({ type: Boolean })
  disabled = false;

  @property({ type: Boolean })
  updatePending = false;

  @property({ attribute: false })
  updateAgent: ManagedAgentKind | undefined;

  @property({ attribute: false }) profiles: ExternalAgentProfile[] = [];
  @state() private managedSelection: ManagedAgentKind = "claude";
  @state() private editingId: string | undefined;
  @state() private argumentDrafts: Record<string, string> = {};
  @state() private drafts: Record<string, ExternalAgentProfile> = {};
  @state() private activeHelp: string | undefined;
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
          this.argumentDrafts = {
            ...this.argumentDrafts,
            [profile.id]: formatExternalAgentArguments(profile.args),
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
    this.argumentDrafts = Object.fromEntries(
      profiles.map((profile) => [profile.id, formatExternalAgentArguments(profile.args)]),
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
    this.editDraft({ command: path });
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
    this.argumentDrafts = { ...this.argumentDrafts, [id]: "" };
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
      this.updatePending ||
      isAgentRuntimeActive(this.runtime.stage) ||
      ["checking", "authenticating", "signing_out"].includes(this.selection.stage);
    const methods =
      this.selection.stage === "authentication_required"
        ? this.selection.auth_methods.filter((method) => method.supported)
        : [];
    const selected = selectedAgent(this.selection);
    const selectedLabel = selected ? agentLabel(selected, this.profiles) : undefined;
    const draft = this.editingId ? this.drafts[this.editingId] : undefined;
    const saved = this.profiles.find((p) => p.id === this.editingId);
    const editorAgent = this.editingId ? { external: this.editingId } : this.managedSelection;
    const selectionMatchesEditor = sameAgent(this.selection.candidate, editorAgent);
    const displayedRuntime = sameAgent(this.runtime.agent, editorAgent) ? this.runtime : undefined;
    const updateForEditor =
      this.updatePending && this.updateAgent === this.managedSelection && !this.editingId;
    const installed = Boolean(displayedRuntime?.current_version);
    const total = displayedRuntime?.total_bytes;
    const validationError = this.commandError();
    const dirty = Boolean(
      draft && (JSON.stringify(draft) !== JSON.stringify(saved) || validationError),
    );

    return html` <section class="settings-group" aria-labelledby="agent-heading">
      <h2 id="agent-heading">Agent</h2>
      <fieldset class="external-executable-settings" ?disabled=${controlsDisabled}>
        <legend class="visually-hidden">Agent to use</legend>
        <div class="settings-field">
          <lens-select
            label="Agent"
            .disabled=${controlsDisabled}
            .value=${this.editingId ?? this.managedSelection}
            .options=${[
              { value: "claude", label: "Claude", icon: agentIcon("Claude") },
              { value: "codex", label: "Codex", icon: agentIcon("Codex") },
              ...Object.values(this.drafts).map((profile) => ({
                value: profile.id,
                icon: agentIcon(profile.name),
                label: `${profile.name || "New preset"}${this.profiles.some((saved) => saved.id === profile.id) ? (Object.values(this.drafts).filter((other) => other.name === profile.name).length > 1 ? ` — ${profile.command}` : "") : " (unsaved)"}`,
              })),
            ]}
            @change=${(event: Event) => {
              const value = (event.target as LensSelect).value;
              this.draftRevision += 1;
              this.activeHelp = undefined;
              this.editingId = value === "claude" || value === "codex" ? undefined : value;
              if (!this.editingId) this.managedSelection = value as ManagedAgentKind;
              if (!this.editingId) this.emit({ type: "select", agent: value as ManagedAgentKind });
              else if (this.profiles.some((profile) => profile.id === value))
                this.emit({ type: "select", agent: { external: value } });
            }}
          ></lens-select>
        </div>
      </fieldset>
      <div class="agent-actions preset-add-actions">
        ${this.help(
          "agent-add-help",
          "Create connection settings for another agent.",
          html` <button
            aria-describedby="agent-add-help"
            ?disabled=${controlsDisabled || Object.keys(this.drafts).length >= 16}
            @click=${() => this.addProfile()}
          >
            Add preset
          </button>`,
        )}
        ${
          draft
            ? this.help(
                "agent-delete-help",
                saved ? "Remove the selected preset." : "Remove this unsaved preset.",
                html` <button
                  aria-describedby="agent-delete-help"
                  ?disabled=${controlsDisabled}
                  @click=${() => (saved ? this.emit({ type: "delete-external-agent", id: draft.id }) : this.discardDraft(draft.id))}
                >
                  ${saved ? "Delete preset" : "Discard draft"}
                </button>`,
              )
            : nothing
        }
      </div>
      ${
        draft
          ? html` <fieldset class="agent-preset-settings" ?disabled=${controlsDisabled}>
              <legend class="visually-hidden">Preset settings</legend>
              <label class="settings-field"
                ><span>Display name</span>
                <input
                  class="external-executable-field"
                  aria-label="Connection name"
                  .value=${draft.name}
                  @input=${(event: Event) => this.editDraft({ name: (event.target as HTMLInputElement).value })}
                />
              </label>
              <div class="settings-field">
                <label for="agent-executable">Executable</label>
                <div class="executable-row">
                  ${this.help(
                    "agent-executable-help",
                    "Enter a command name or an executable path. Paths do not need quotes.",
                    html` <input
                      id="agent-executable"
                      class="external-executable-field"
                      aria-label="Executable"
                      aria-describedby="agent-executable-help"
                      spellcheck="false"
                      autocomplete="off"
                      placeholder="goose"
                      .value=${draft.command}
                      @input=${(event: Event) => this.editDraft({ command: (event.target as HTMLInputElement).value })}
                    />`,
                  )}
                  ${this.help(
                    "agent-choose-help",
                    "Choose an executable file.",
                    html` <button
                      aria-describedby="agent-choose-help"
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
                      Choose…
                    </button>`,
                  )}
                </div>
              </div>
              <label class="settings-field"
                ><span>Arguments</span>
                ${this.help(
                  "agent-arguments-help",
                  "Separate arguments with spaces; quote values containing spaces. No shell expansion.",
                  html` <input
                    class="external-executable-field"
                    aria-label="Arguments"
                    aria-describedby="agent-arguments-help"
                    spellcheck="false"
                    autocomplete="off"
                    placeholder="acp"
                    .value=${this.argumentDrafts[draft.id] ?? ""}
                    @input=${(event: Event) => this.editArguments((event.target as HTMLInputElement).value)}
                  />`,
                )}
              </label>
              <p class="help">Install and update this agent’s CLI separately.</p>
              ${validationError ? html`<p role="alert">${validationError}</p>` : nothing}
              ${dirty ? html`<p class="help" role="status">Unsaved connection changes have not been verified.</p>` : nothing}
              <div class="agent-actions">
                ${this.help(
                  "agent-save-help",
                  "Save these settings and start the executable to verify its ACP connection.",
                  html` <button
                    aria-describedby="agent-save-help"
                    ?disabled=${!draft.name.trim() || Boolean(validationError)}
                    @click=${() =>
                      this.emit({
                        type: "save-external-agent",
                        profile: {
                          id: draft.id,
                          name: draft.name,
                          command: draft.command,
                          arguments: this.argumentDrafts[draft.id] ?? "",
                        },
                      })}
                  >
                    Save and Verify
                  </button>`,
                )}
              </div>
            </fieldset>`
          : nothing
      }
      <div class="agent-status" aria-live="polite">
        <div class="agent-status-row installation-status">
          <span class="agent-status-label">Installation</span>
          <div class="runtime-status">
            <output
              class=${displayedRuntime?.stage === "failed" && !updateForEditor ? "status-warning" : "runtime-message"}
            >
              ${
                updateForEditor &&
                (!displayedRuntime || !isAgentRuntimeActive(displayedRuntime.stage))
                  ? `Checking ${agentLabel(this.managedSelection)} for updates…`
                  : (displayedRuntime?.error ??
                    displayedRuntime?.message ??
                    (displayedRuntime
                      ? AGENT_RUNTIME_LABEL[displayedRuntime.stage]
                      : draft
                        ? "Managed separately"
                        : "Not checked"))
              }
            </output>
            ${
              displayedRuntime?.stage === "downloading"
                ? total === undefined
                  ? html`<progress aria-label="Agent runtime download progress"></progress>`
                  : html`<progress
                      aria-label="Agent runtime download progress"
                      .value=${displayedRuntime.downloaded_bytes}
                      max=${total}
                    ></progress>`
                : nothing
            }
          </div>
          ${
            !draft
              ? html`<div class="managed-agent-actions">
                  ${this.help(
                    "agent-update-help",
                    installed
                      ? "Check for updates and install the latest available version."
                      : "Download and install the selected agent.",
                    html` <button
                      aria-describedby="agent-update-help"
                      ?disabled=${controlsDisabled}
                      @click=${() => this.requestManagedUpdate()}
                    >
                      ${installed ? "Update" : "Install"}
                    </button>`,
                  )}
                </div>`
              : nothing
          }
        </div>
        <div class="agent-status-row">
          <span class="agent-status-label">Connection</span>
          <output
            class="selection-status ${selectionMatchesEditor && !dirty && this.selection.stage === "selected" ? "status-ok" : "status-warning"}"
          >
            ${
              !selectionMatchesEditor
                ? draft
                  ? "Save and verify to connect"
                  : AGENT_SELECTION_LABEL.unselected
                : dirty
                  ? "Saved connection; changes have not been verified"
                  : (this.selection.error ??
                    this.selection.message ??
                    AGENT_SELECTION_LABEL[this.selection.stage])
            }
          </output>
        </div>
      </div>
      <p class="help">Credentials are managed by the selected agent.</p>
      ${
        selectionMatchesEditor && this.selection.stage === "authentication_required" && !draft
          ? html`<div class="agent-actions" aria-label="Agent authentication">
              ${
                methods.length
                  ? methods.map(
                      (method) => html` <button
                        ?disabled=${this.disabled}
                        @click=${() => this.emit({ type: "authenticate", methodId: method.id })}
                      >
                        Authenticate with ${method.name}…
                      </button>`,
                    )
                  : html`<p>Authenticate with this Agent's existing CLI, then select it again.</p>`
              }
            </div>`
          : nothing
      }
      ${
        !draft &&
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
        selectionMatchesEditor && selected && this.selection.supports_logout
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
      <div class="agent-actions agent-reset-actions">
        ${this.help(
          "agent-reset-help",
          "Restore preset defaults after confirmation.",
          html` <button
            aria-describedby="agent-reset-help"
            ?disabled=${controlsDisabled}
            @click=${() => this.emit({ type: "reset-agent-presets" })}
          >
            Reset presets…
          </button>`,
        )}
      </div>
    </section>`;
  }

  private requestManagedUpdate(): void {
    if (this.editingId) return;
    this.emit({ type: "update-managed-agent", agent: this.managedSelection });
  }

  private editArguments(value: string): void {
    if (!this.editingId) return;
    this.argumentDrafts = { ...this.argumentDrafts, [this.editingId]: value };
    this.draftRevision += 1;
    try {
      this.editDraft({ args: parseExternalAgentArguments(value) });
    } catch {
      /* Preserve incomplete argument text until the user completes it. */
    }
  }
  private commandError(): string | undefined {
    if (!this.editingId) return undefined;
    const command = this.drafts[this.editingId]?.command ?? "";
    if (!command.trim()) return "Enter an executable name or path.";
    if (/[\r\n\0]/.test(command)) return "Use an executable without line breaks or NUL characters.";
    try {
      parseExternalAgentArguments(this.argumentDrafts[this.editingId] ?? "");
      return undefined;
    } catch (error) {
      return (error as Error).message;
    }
  }

  private help(id: string, text: string, control: unknown) {
    return html`<span
      class="agent-help"
      @pointerenter=${() => {
        this.activeHelp = id;
      }}
      @pointerleave=${() => {
        this.activeHelp = undefined;
      }}
      @focusin=${() => {
        this.activeHelp = id;
      }}
      @focusout=${() => {
        this.activeHelp = undefined;
      }}
      @keydown=${(event: KeyboardEvent) => {
        if (event.key === "Escape") this.activeHelp = undefined;
      }}
      >${control}<span id=${id} role="tooltip" ?hidden=${this.activeHelp !== id}
        >${text}</span
      ></span
    >`;
  }

  private discardDraft(id: string): void {
    const { [id]: _removed, ...remaining } = this.drafts;
    const { [id]: _arguments, ...remainingArguments } = this.argumentDrafts;
    this.drafts = remaining;
    this.argumentDrafts = remainingArguments;
    const candidate = this.selection?.candidate;
    this.editingId = typeof candidate === "object" ? candidate.external : undefined;
    if (typeof candidate === "string") this.managedSelection = candidate;
    this.draftRevision += 1;
  }

  private emit(intent: AgentIntent): void {
    dispatchComponentEvent(this, AGENT_INTENT_EVENT, intent);
  }
}
