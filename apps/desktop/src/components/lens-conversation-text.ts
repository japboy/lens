import { LitElement, css, html, type PropertyValues } from "lit";
import { customElement, property } from "lit/decorators.js";

const MAX_CHUNK_CHARACTERS = 4096;
const MAX_CHUNK_LINES = 32;

/** Lossless display chunks bound layout work even for a single unbroken paragraph. */
export function conversationTextChunks(text: string): string[] {
  const chunks: string[] = [];
  let start = 0;
  while (start < text.length) {
    let end = Math.min(start + MAX_CHUNK_CHARACTERS, text.length);
    // Keep Unicode pairs and Windows newlines within the same layout chunk.
    if (end < text.length) {
      const previous = text.charCodeAt(end - 1);
      const next = text.charCodeAt(end);
      if (
        (previous >= 0xd800 && previous <= 0xdbff && next >= 0xdc00 && next <= 0xdfff) ||
        (previous === 13 && next === 10)
      )
        end--;
    }
    let lines = 0;
    for (let index = start; index < end; index++) {
      const character = text.charCodeAt(index);
      if (character !== 10 && character !== 13) continue;
      if (character === 13 && text.charCodeAt(index + 1) === 10) index++;
      if (++lines === MAX_CHUNK_LINES) {
        end = index + 1;
        break;
      }
    }
    chunks.push(text.slice(start, end));
    start = end;
  }
  return chunks;
}

/** Diagnostic text only: no parsing, links, media decoding, or executable HTML. */
@customElement("lens-conversation-text")
export class LensConversationText extends LitElement {
  @property({ type: String }) text = "";
  private chunks: string[] = [];
  static styles = css`
    :host {
      display: block;
      min-width: 0;
      font-family: ui-monospace, monospace;
      font-size: 0.875em;
      line-height: 1.5;
    }
    .chunk {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      content-visibility: auto;
      contain-intrinsic-block-size: auto 24em;
    }
  `;
  protected willUpdate(changed: PropertyValues<this>): void {
    if (changed.has("text")) this.chunks = conversationTextChunks(this.text);
  }
  protected render() {
    const chunks = this.chunks.map((chunk) => html`<div class="chunk">${chunk}</div>`);
    return html`<div class="text">${chunks}</div>`;
  }
}
