import { LitElement, css, html } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { keyed } from "lit/directives/keyed.js";
import type { AboutInfo } from "../application/webview-port";

const appIconUrl = new URL("../../src-tauri/icons/128x128@2x.png", import.meta.url).href;

export type AboutState =
  | { stage: "loading" }
  | { stage: "ready"; info: AboutInfo }
  | { stage: "failed"; message: string };

@customElement("lens-about-view")
export class LensAboutView extends LitElement {
  static styles = css`
    :host {
      display: block;
      height: 100%;
      color: CanvasText;
      background: Canvas;
    }
    main {
      box-sizing: border-box;
      height: 100%;
      padding: 24px;
      display: grid;
      grid-template-rows: auto auto minmax(0, 1fr);
      gap: 18px;
    }
    header {
      display: grid;
      grid-template-columns: 96px minmax(0, auto);
      align-items: center;
      gap: 20px;
      justify-self: center;
      max-width: 100%;
    }
    .app-icon {
      width: 96px;
      height: 96px;
      object-fit: contain;
    }
    h1 {
      margin: 0 0 8px;
      font-size: 24px;
    }
    p {
      margin: 4px 0;
    }
    .documents {
      display: flex;
      align-items: center;
      gap: 12px;
    }
    select {
      font: inherit;
    }
    textarea {
      box-sizing: border-box;
      width: 100%;
      height: 100%;
      min-height: 0;
      resize: none;
      padding: 12px;
      font:
        12px/1.55 ui-monospace,
        monospace;
      color: CanvasText;
      background: Field;
      border: 1px solid GrayText;
      border-radius: 4px;
    }
  `;

  @property({ attribute: false }) model: AboutState = { stage: "loading" };
  @state() private document: "license" | "notice" = "license";

  protected render() {
    const model = this.model;
    if (model.stage === "loading") return html`<main><p role="status">Loading About…</p></main>`;
    if (model.stage === "failed")
      return html`<main><p role="alert">Unable to load About: ${model.message}</p></main>`;
    const info = model.info;
    const label = this.document === "license" ? "LICENSE" : "NOTICE";
    return html`<main aria-label="About">
      <header>
        <img class="app-icon" src=${appIconUrl} alt="" width="96" height="96" />
        <div>
          <h1>${info.name}</h1>
          <p>Version ${info.version}</p>
          <p>${info.copyright}</p>
        </div>
      </header>
      <div class="documents">
        <label for="document">License documents</label>
        <select id="document" .value=${this.document} @change=${this.selectDocument}>
          <option value="license">LICENSE</option>
          <option value="notice">NOTICE</option>
        </select>
      </div>
      ${keyed(this.document, html`<textarea aria-label=${label} readonly spellcheck="false" wrap="soft" .value=${info[this.document]}></textarea>`)}
    </main>`;
  }

  private selectDocument = (event: Event): void => {
    const value = (event.target as HTMLSelectElement).value;
    if (value === "license" || value === "notice") this.document = value;
  };
}
