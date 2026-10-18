import { css, html, LitElement, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import { styleMap } from "lit/directives/style-map.js";
import type { HistoryIntent } from "../../contracts/events";
import type { HistoryEntry, HistoryPresentation } from "../../contracts/view-models";
import { controlStyles, accessibilityStyles } from "../../styles/component-styles";
import { agentIcon } from "../../presentation/agent-icons";

const historyDateFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: "short",
  timeStyle: "short",
});

/** Cache presentation only; native commands and confirmation dialogs belong to the page. */
@customElement("lens-session-history")
export class LensSessionHistory extends LitElement {
  @property({ attribute: false }) history: HistoryPresentation = {
    stage: "inactive",
    offset: 0,
    operation: "idle",
  };
  @property({ type: Boolean }) disabled = false;

  static styles = [
    controlStyles,
    accessibilityStyles,
    css`
      :host {
        display: block;
        min-width: 0;
      }
      .toolbar,
      .actions,
      .pagination {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
      }
      .toolbar,
      .pagination {
        justify-content: space-between;
      }
      .toolbar {
        margin-bottom: 12px;
      }
      .list {
        border: 1px solid var(--settings-group-border, GrayText);
        border-radius: 8px;
        overflow: hidden;
        background: var(--settings-group-background, Canvas);
      }
      .row {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 14px 16px;
        border-bottom: 1px solid var(--settings-group-border, GrayText);
      }
      .row:last-child {
        border-bottom: 0;
      }
      .content {
        flex: 1;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      .title {
        font-weight: 500;
      }
      .metadata,
      .note,
      .pagination {
        font-size: 12px;
        color: GrayText;
      }
      .metadata {
        margin-top: 4px;
      }
      .agent-label {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        max-width: 100%;
        vertical-align: middle;
      }
      .agent-name {
        min-width: 0;
      }
      .agent-icon {
        display: inline-block;
        width: 16px;
        height: 16px;
        flex: 0 0 16px;
        background: currentColor;
        mask: no-repeat center / contain;
        -webkit-mask: no-repeat center / contain;
      }
      .pagination {
        margin-top: 12px;
      }
      .note {
        margin-top: 20px;
      }
      .result {
        margin: 0 0 14px;
        padding: 12px 14px;
        border: 1px solid var(--settings-group-border, GrayText);
        border-radius: 8px;
        background: var(--settings-group-background, Canvas);
        overflow-wrap: anywhere;
      }
      .result p {
        margin: 4px 0 0;
      }
      .result details {
        margin-top: 8px;
      }
      .result ul {
        padding-left: 20px;
      }
      .error {
        color: var(--status-error-color, MarkText);
      }
      .empty {
        padding: 36px 20px;
        text-align: center;
        color: GrayText;
      }
      @media (max-width: 420px) {
        .row {
          flex-direction: column;
          align-items: stretch;
        }
        .row > .actions {
          justify-content: flex-end;
        }
      }
    `,
  ];

  private emit(intent: HistoryIntent): void {
    this.dispatchEvent(
      new CustomEvent<HistoryIntent>("lens-history-intent", {
        detail: intent,
        bubbles: true,
        composed: true,
      }),
    );
  }

  private row(entry: HistoryEntry, busy: boolean) {
    const timestamp = entry.updated_at ? Date.parse(entry.updated_at) : Number.NaN;
    const presence = !entry.available
      ? "Original Agent settings unavailable"
      : entry.state === "listed"
        ? "Listed by Agent"
        : entry.state === "local_only"
          ? "Saved in Lens"
          : "Not seen in the last Agent listing";
    return html`<div class="row">
      <div class="content">
        <div class="title">${entry.title}</div>
        <div class="metadata">
          <span class="agent-label"
            ><span
              class="agent-icon"
              aria-hidden="true"
              style=${styleMap({ "mask-image": `url(${JSON.stringify(agentIcon(entry.agent))})`, "-webkit-mask-image": `url(${JSON.stringify(agentIcon(entry.agent))})` })}
            ></span
            ><span class="agent-name">${entry.agent}</span></span
          >
          ·
          ${
            Number.isFinite(timestamp)
              ? html`<time datetime=${entry.updated_at!}
                  >${historyDateFormatter.format(timestamp)}</time
                >`
              : "Unknown date"
          }
        </div>
        <div class="metadata">${entry.cwd}</div>
        <div class="metadata">${presence}</div>
      </div>
      <div class="actions">
        <button
          data-lens-button-role="normal"
          type="button"
          aria-label=${`Open ${entry.title}`}
          ?disabled=${busy || !entry.available}
          @click=${() => this.emit({ type: "open-history", id: entry.id })}
        >
          Open
        </button>
        <button
          data-lens-button-role="destructive"
          type="button"
          aria-label=${`Delete ${entry.title}`}
          ?disabled=${busy}
          @click=${() => this.emit({ type: "delete-history", id: entry.id })}
        >
          Delete…
        </button>
      </div>
    </div>`;
  }

  protected render() {
    const state = this.history;
    const page = state.page;
    const busy = this.disabled || state.stage === "loading" || state.operation !== "idle";
    const ready = state.stage === "ready";
    const outcome = state.outcome;
    const result = outcome?.result;
    const pending =
      state.operation === "synchronizing"
        ? "Syncing with Agents…"
        : state.operation === "deleting"
          ? "Deleting history…"
          : state.operation === "opening"
            ? "Opening session history…"
            : state.operation === "confirming"
              ? "Awaiting confirmation…"
              : state.stage === "loading"
                ? "Loading saved history…"
                : undefined;
    return html` <div class="toolbar">
        <span
          >${page ? `${page.total} saved ${page.total === 1 ? "session" : "sessions"}` : "Saved sessions"}</span
        >
        <div class="actions">
          <button
            data-lens-button-role="normal"
            type="button"
            ?disabled=${busy}
            @click=${() => this.emit({ type: "sync-history" })}
          >
            Sync with Agents
          </button>
          <button
            data-lens-button-role="destructive"
            type="button"
            ?disabled=${busy || !ready || !page?.total}
            @click=${() => this.emit({ type: "delete-all-history" })}
          >
            Delete All…
          </button>
        </div>
      </div>
      ${pending ? html`<p role="status" aria-live="polite">${pending}</p>` : nothing}
      ${
        state.error || state.readError
          ? html`<div class="result">
              ${state.error ? html`<p class="error" role="alert">${state.error}</p>` : nothing}
              ${state.readError ? html`<p class="error" role="alert">Unable to refresh the history list: ${state.readError}</p>` : nothing}
              <button
                data-lens-button-role="normal"
                type="button"
                ?disabled=${busy}
                @click=${() => this.emit({ type: "list-history", offset: state.offset })}
              >
                Refresh List
              </button>
            </div>`
          : nothing
      }
      ${
        result
          ? html`<div class="result" role="status" aria-live="polite">
              <strong
                >${outcome!.kind === "sync" ? (result.failed ? "Sync completed with failures" : "Sync complete") : result.failed ? "Some sessions could not be deleted" : "Session history deleted"}</strong
              >
              <p>
                ${
                  outcome!.kind === "sync"
                    ? `${result.succeeded} Agent/workspace combinations refreshed · ${result.purged} removed from Lens · ${result.failed} failed or incomplete`
                    : `${result.succeeded} sessions deleted · ${result.failed} could not be deleted`
                }
              </p>
              ${
                result.errors.length
                  ? html`<details>
                      <summary>View details</summary>
                      <ul>
                        ${result.errors.map((error) => html`<li>${error}</li>`)}
                      </ul>
                    </details>`
                  : nothing
              }
            </div>`
          : nothing
      }
      <div class="list" aria-label="Saved session history" aria-busy=${busy ? "true" : "false"}>
        ${page?.entries.map((entry) => this.row(entry, busy || !ready))}
        ${ready && !page?.total ? html`<div class="empty">No saved session history.</div>` : nothing}
      </div>
      <div class="pagination">
        <span
          >${page ? (page.total ? `${state.offset + 1}–${Math.min(state.offset + page.entries.length, page.total)} of ${page.total}` : "0 sessions") : nothing}</span
        >
        <div class="actions">
          <button
            data-lens-button-role="normal"
            type="button"
            ?disabled=${busy || !ready || state.offset === 0}
            @click=${() => this.emit({ type: "list-history", offset: state.offset - 50 })}
          >
            Previous
          </button>
          <button
            data-lens-button-role="normal"
            type="button"
            ?disabled=${busy || !ready || !page || state.offset + page.entries.length >= page.total}
            @click=${() => this.emit({ type: "list-history", offset: state.offset + 50 })}
          >
            Next
          </button>
        </div>
      </div>
      <p class="note">
        Sync checks every Agent and working directory recorded in Lens. History whose original Agent
        settings are no longer available is removed from Lens. Temporary connection failures keep
        the saved history.
      </p>`;
  }
}
