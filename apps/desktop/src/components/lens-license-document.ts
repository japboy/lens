import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property } from "lit/decorators.js";
import type { AboutDocuments } from "../application/webview-port";

export type AboutResource<T> =
  | { stage: "loading" }
  | { stage: "ready"; value: T }
  | { stage: "failed"; message: string };

@customElement("lens-license-document")
export class LensLicenseDocument extends LitElement {
  @property({ attribute: false }) model: AboutResource<AboutDocuments> = { stage: "loading" };
  @property() document: "license" | "notice" = "license";
  protected createRenderRoot(): HTMLElement {
    return this;
  }
  protected willUpdate(changed: PropertyValues<this>): void {
    this.setAttribute("aria-busy", String(this.model.stage === "loading"));
    this.setAttribute("aria-label", this.document === "license" ? "LICENSE" : "NOTICE");
    if (changed.has("document")) this.scrollTop = 0;
  }
  protected render() {
    if (this.model.stage === "failed")
      return html`<p role="alert">Unable to load license documents: ${this.model.message}</p>`;
    if (this.model.stage === "loading") return nothing;
    return this.documentChunks(this.model.value[this.document]).map(
      (chunk) => html`<span class="document-chunk">${chunk}</span>`,
    );
  }
  private documentChunks(text: string): string[] {
    const lines = text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
    const chunks: string[] = [];
    for (let start = 0; start < lines.length; start += 32)
      chunks.push(lines.slice(start, start + 32).join(""));
    return chunks;
  }
}
