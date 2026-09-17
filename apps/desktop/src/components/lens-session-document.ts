import { LitElement, css, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import "@lit-labs/virtualizer";
import type { LitVirtualizer } from "@lit-labs/virtualizer/LitVirtualizer.js";
import type { VisibilityChangedEvent } from "@lit-labs/virtualizer/events.js";
import type { DocumentBlock, SessionDocument } from "../application/session-document";
import {
  ConversationRenderCache,
  type BlockLoader,
  type PreparedConversationBlock,
} from "../application/conversation-render-cache";
import { imageDataUrl } from "../view-model";
import { externalMarkdownUrl } from "../markdown";
import { dispatchComponentEvent, OVERLAY_INTENT_EVENT } from "./events";
import "./lens-conversation-markdown";

export interface ConversationRow {
  id: string;
  entryId: string;
  title?: string;
  status?: string;
  block?: DocumentBlock;
  contentKey: string;
}
const inlineVersions = new WeakMap<object, number>();
let nextInlineVersion = 0;
function contentVersion(block: object): number {
  if ("type" in block && block.type === "deferred")
    return (block as Extract<DocumentBlock, { type: "deferred" }>).revision;
  let version = inlineVersions.get(block);
  if (version === undefined) {
    version = ++nextInlineVersion;
    inlineVersions.set(block, version);
  }
  return version;
}
/** Metadata only: this does not parse Markdown/HTML or decode images. */
export function conversationRows(document: SessionDocument | undefined): ConversationRow[] {
  return (
    document?.entries.flatMap((entry) => {
      const rows: ConversationRow[] = [
        {
          id: `${entry.id}:header`,
          entryId: entry.id,
          title: entry.kind === "message" ? entry.role : entry.title,
          status: entry.kind === "tool" ? entry.status : undefined,
          contentKey: `${entry.id}:header`,
        },
      ];
      entry.blocks.forEach((block, index) =>
        rows.push({
          id: `${entry.id}:${index}`,
          entryId: entry.id,
          block,
          contentKey: `${entry.id}:${index}:${contentVersion(block)}`,
        }),
      );
      if (entry.kind === "tool" && entry.accepted_html)
        rows.push({
          id: `${entry.id}:accepted`,
          entryId: entry.id,
          block: { type: "html", text: entry.accepted_html },
          contentKey: `${entry.id}:accepted:${contentVersion(entry)}`,
        });
      return rows;
    }) ?? []
  );
}
interface ReadingPosition {
  rowId: string;
  offset: number;
  followTail: boolean;
}
// One active session survives tab replacement; a generation change invalidates all state.
const retained = {
  identity: "",
  position: undefined as ReadingPosition | undefined,
  cache: new ConversationRenderCache(),
};

/** One variable-height, block-virtualized renderer for live and restored conversations. */
@customElement("lens-session-document")
export class LensSessionDocument extends LitElement {
  @property({ attribute: false }) document: SessionDocument | undefined;
  @property({ type: String }) identity = "session";
  @property({ attribute: false }) loadBlock: BlockLoader | undefined;
  private rows: ConversationRow[] = [];
  private firstVisible = 0;
  private restoring = false;
  private restoreOnUpdate = false;
  private restorationRevision = 0;
  static styles = css`
    :host {
      display: block;
      min-height: 0;
      height: 100%;
      overflow: hidden;
      overflow-wrap: anywhere;
    }
    lit-virtualizer {
      display: block;
      height: 100%;
      min-height: 0;
      overflow: auto;
    }
    .heading {
      box-sizing: border-box;
      min-height: 40px;
      padding: 16px 16px 4px;
      font: inherit;
      font-weight: 600;
    }
    .status {
      font-size: 0.85em;
      font-weight: normal;
      opacity: 0.7;
    }
    lens-conversation-block {
      display: block;
      min-height: 24px;
      padding: 0 16px 12px;
      box-sizing: border-box;
    }
  `;
  connectedCallback(): void {
    super.connectedCallback();
    if (this.hasUpdated && retained.identity === this.identity) {
      this.restoreOnUpdate = Boolean(retained.position);
      this.requestUpdate();
    }
  }
  disconnectedCallback(): void {
    this.capturePosition();
    this.restorationRevision++;
    this.restoring = false;
    if (retained.identity === this.identity) retained.cache.suspend();
    super.disconnectedCallback();
  }
  protected willUpdate(changed: PropertyValues<this>): void {
    if (retained.identity !== this.identity) {
      retained.cache.clear();
      retained.identity = this.identity;
      retained.position = undefined;
      this.firstVisible = 0;
      this.restoreOnUpdate = false;
    } else if (changed.has("document")) this.capturePosition();
    if (changed.has("document") || changed.has("identity")) {
      this.rows = conversationRows(this.document);
      this.restoreOnUpdate = Boolean(retained.position);
    }
  }
  protected updated(): void {
    if (!this.restoreOnUpdate) return;
    this.restoreOnUpdate = false;
    const position = retained.position;
    const virtualizer = this.virtualizer;
    if (!position || !virtualizer) return;
    const index = position.followTail
      ? this.rows.length - 1
      : this.rows.findIndex((row) => row.id === position.rowId);
    if (index < 0) return;
    const identity = this.identity;
    const revision = ++this.restorationRevision;
    this.restoring = true;
    virtualizer.element(index)?.scrollIntoView({ block: position.followTail ? "end" : "start" });
    void virtualizer.layoutComplete
      ?.then(() => {
        if (
          !this.isConnected ||
          this.identity !== identity ||
          this.restorationRevision !== revision
        )
          return;
        if (!position.followTail) virtualizer.scrollTop -= position.offset;
      })
      .catch(() => {
        // Virtualizer cancels layoutComplete when a cached tab disconnects.
      })
      .finally(() => {
        if (this.restorationRevision === revision) this.restoring = false;
      });
  }
  private get virtualizer(): LitVirtualizer<ConversationRow> | null {
    return this.renderRoot.querySelector(
      "lit-virtualizer",
    ) as LitVirtualizer<ConversationRow> | null;
  }
  private capturePosition = (): void => {
    if (this.restoring || retained.identity !== this.identity) return;
    const virtualizer = this.virtualizer;
    const row = this.rows[this.firstVisible];
    if (!virtualizer || !row || !virtualizer.clientHeight) return;
    const element = [...virtualizer.children].find(
      (child) => child.getAttribute("data-row-id") === row.id,
    );
    const offset = element
      ? element.getBoundingClientRect().top - virtualizer.getBoundingClientRect().top
      : 0;
    retained.position = {
      rowId: row.id,
      offset,
      followTail: virtualizer.scrollHeight - virtualizer.scrollTop - virtualizer.clientHeight <= 2,
    };
  };
  // Scroll events also come from our own restoration. Cancel only on input intent,
  // before the browser performs the user's scroll, then let scroll record its result.
  private cancelRestorationOnInput = (event: Event): void => {
    if (event instanceof KeyboardEvent) {
      if (!["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "].includes(event.key))
        return;
      if (
        event
          .composedPath()
          .some(
            (target) =>
              target instanceof HTMLElement &&
              (target.isContentEditable ||
                ["INPUT", "TEXTAREA", "SELECT", "BUTTON"].includes(target.tagName)),
          )
      )
        return;
    }
    this.restorationRevision++;
    this.restoring = false;
    this.restoreOnUpdate = false;
  };
  private visibilityChanged = (event: VisibilityChangedEvent): void => {
    this.firstVisible = Math.max(0, event.first);
    this.capturePosition();
  };
  protected render() {
    return this.rows.length
      ? html`<lit-virtualizer
          scroller
          role="log"
          aria-live="off"
          .items=${this.rows}
          .keyFunction=${(row: ConversationRow) => row.id}
          .renderItem=${this.renderRow}
          @visibilityChanged=${this.visibilityChanged}
          @scroll=${this.capturePosition}
          @wheel=${this.cancelRestorationOnInput}
          @touchstart=${this.cancelRestorationOnInput}
          @keydown=${this.cancelRestorationOnInput}
        ></lit-virtualizer>`
      : html`<p>No session content is available.</p>`;
  }
  private renderRow = (row: ConversationRow) =>
    row.block
      ? html`<lens-conversation-block
          data-row-id=${row.id}
          data-entry-id=${row.entryId}
          .block=${row.block}
          .contentKey=${`${this.identity}:${row.contentKey}`}
          .cache=${retained.cache}
          .loadBlock=${this.loadBlock}
        ></lens-conversation-block>`
      : html`<div class="heading" data-row-id=${row.id} data-entry-id=${row.entryId}>
          ${row.title}${row.status ? html`<span class="status"> ${row.status}</span>` : nothing}
        </div>`;
}

@customElement("lens-conversation-block")
export class LensConversationBlock extends LitElement {
  @property({ attribute: false }) block: DocumentBlock | undefined;
  @property({ type: String }) contentKey = "";
  @property({ attribute: false }) cache: ConversationRenderCache | undefined;
  @property({ attribute: false }) loadBlock: BlockLoader | undefined;
  @state() private prepared: PreparedConversationBlock | undefined;
  @state() private error: string | undefined;
  private generation = 0;
  private preparation: AbortController | undefined;
  static styles = css`
    :host {
      display: block;
      overflow-wrap: anywhere;
    }
    img {
      max-width: 100%;
      height: auto;
    }
    iframe {
      width: 100%;
      min-height: 360px;
      border: 0;
      background: white;
    }
    .loading {
      min-height: 24px;
      opacity: 0.6;
    }
  `;
  connectedCallback(): void {
    super.connectedCallback();
    if (this.hasUpdated) this.requestUpdate("block", undefined);
  }
  disconnectedCallback(): void {
    this.generation++;
    this.preparation?.abort();
    super.disconnectedCallback();
  }
  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("contentKey") && !changed.has("block") && !changed.has("loadBlock")) return;
    const generation = ++this.generation;
    this.preparation?.abort();
    this.preparation = new AbortController();
    this.prepared = this.cache?.peek(this.contentKey);
    this.error = undefined;
    if (this.prepared || !this.block || !this.cache) return;
    void this.cache
      .resolve(this.contentKey, this.block, this.loadBlock, this.preparation.signal)
      .then((value) => {
        if (this.isConnected && this.generation === generation) this.prepared = value;
      })
      .catch((error) => {
        if (this.isConnected && this.generation === generation) this.error = String(error);
      });
  }
  protected render() {
    if (this.error) return html`<p role="alert">${this.error}</p>`;
    const block = this.prepared?.block;
    if (!block) return html`<div class="loading" role="status">Loading content…</div>`;
    switch (block.type) {
      case "markdown":
        return html`<lens-conversation-markdown
          .markdown=${block.text}
          @click=${this.openLink}
        ></lens-conversation-markdown>`;
      case "image": {
        const source = imageDataUrl(block);
        return source
          ? html`<img src=${source} alt="Session image" />`
          : html`<p>Unsupported image type: ${block.mime_type}</p>`;
      }
      case "html":
        return html`<iframe
            title="Session HTML output"
            sandbox="allow-popups"
            referrerpolicy="no-referrer"
            .srcdoc=${this.prepared?.html?.document ?? ""}
          ></iframe
          >${this.prepared?.html?.notices.map((notice) => html`<p role="note">${notice}</p>`)}`;
      case "unsupported":
        return html`<p role="note">Unsupported content: ${block.content_type}</p>`;
      case "deferred":
        return nothing;
    }
  }
  private openLink = (event: MouseEvent): void => {
    const link = event
      .composedPath()
      .find((node): node is HTMLAnchorElement => node instanceof HTMLAnchorElement);
    if (!link) return;
    event.preventDefault();
    const url = externalMarkdownUrl(link.getAttribute("href") ?? "");
    if (url) dispatchComponentEvent(this, OVERLAY_INTENT_EVENT, { type: "open-external-url", url });
  };
}
