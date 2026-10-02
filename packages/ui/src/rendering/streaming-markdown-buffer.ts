import { protectStreamingMathSource, streamingCommitBoundary } from "./streaming-math";

export const STREAMING_TAIL_LIMIT = 65_536;

/**
 * Owns the undecided source tail. Only complete lines outside unfinished math/code
 * enter Generative DOM, whose consumed bytes cannot be reparsed on later pushes.
 * An oversized tail switches the remaining response to literal DOM text; settlement
 * still reparses the complete authoritative Markdown through the canonical renderer.
 */
export class StreamingMarkdownBuffer {
  private pending = "";
  private literalRemainder = false;
  private readonly tail = document.createElement("span");

  constructor(
    private readonly container: HTMLElement,
    private readonly commit: (source: string) => void,
  ) {
    this.tail.className = "streaming-markdown-tail";
    this.tail.style.whiteSpace = "pre-wrap";
  }

  push(delta: string): void {
    if (this.literalRemainder) {
      this.tail.append(document.createTextNode(delta));
      return;
    }
    // Bound temporary source storage even when one ACP update is very large.
    for (let offset = 0; offset < delta.length; offset += STREAMING_TAIL_LIMIT) {
      const chunk = delta.slice(offset, offset + STREAMING_TAIL_LIMIT);
      if (this.literalRemainder) {
        this.tail.append(document.createTextNode(chunk));
        continue;
      }
      const source = this.pending + chunk;
      const boundary = streamingCommitBoundary(source);
      this.tail.remove();
      if (boundary > 0) this.commit(protectStreamingMathSource(source.slice(0, boundary)));
      this.pending = source.slice(boundary);
      this.tail.textContent = this.pending;
      if (this.pending.length > STREAMING_TAIL_LIMIT) {
        this.literalRemainder = true;
        this.pending = "";
        this.tail.dataset.streamingFallback = "capacity";
      }
      if (this.tail.textContent) this.container.append(this.tail);
    }
  }

  destroy(): void {
    this.pending = "";
    this.tail.remove();
  }
}
