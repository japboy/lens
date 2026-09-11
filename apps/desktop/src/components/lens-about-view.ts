import { LitElement, html, nothing, css } from "lit";
import { viewHostStyles } from "../styles/component-styles";
import { customElement, property, state } from "lit/decorators.js";
import type { AboutDocuments, AboutInfo } from "../application/webview-port";
import {
  initialAboutState,
  documentKind,
  type Resource,
  type DocumentKind,
} from "../rendering/initial-state";
import icon from "../../src-tauri/icons/128x128@2x.png";

@customElement("lens-about-view")
export class LensAboutView extends LitElement {
  static styles = [
    viewHostStyles,
    css`
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
      grid-template-columns: 96px minmax(0, 1fr);
      align-items: center;
      gap: 20px;
      justify-self: center;
      width: min(100%, 340px);
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
      min-height: 1lh;
    }
    .documents {
      display: flex;
      align-items: center;
      gap: 12px;
    }
    .documents select {
      font: inherit;
    }
    .document-region {
      display: grid;
      grid-template-rows: auto minmax(0, 1fr);
      min-height: 0;
    }
    lens-license-document {
      grid-row: 2;
      display: block;
      box-sizing: border-box;
      width: 100%;
      height: 100%;
      min-width: 0;
      min-height: 0;
      overflow: auto;
      padding: 12px;
      font:
        12px/1.55 ui-monospace,
        monospace;
      color: CanvasText;
      background: Field;
      border: 1px solid GrayText;
      border-radius: 4px;
    }
    lens-license-document:focus-visible {
      outline: 2px solid Highlight;
      outline-offset: -2px;
    }
    .document-chunk {
      display: block;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      content-visibility: auto;
      contain-intrinsic-block-size: auto 32lh;
    }
  `,
  ];
  @property({ attribute: false }) info: Resource<AboutInfo> = initialAboutState().info;
  @property({ attribute: false }) documents: Resource<AboutDocuments> =
    initialAboutState().documents;
  @state() private selectedDocument: DocumentKind = initialAboutState().document;

  adoptNativeSelection(): void {
    const select = this.shadowRoot?.querySelector("select");
    if (!select) throw new Error("Missing native About document select");
    this.selectedDocument = documentKind(select.value);
  }

  protected render() {
    const info = this.info.stage === "ready" ? this.info.value : undefined;
    return html`
      <main aria-label="About">
        <header>
          <img class="app-icon" src=${icon} alt="" width="96" height="96" />
          <div>
            <h1 data-about-name>${info?.name ?? "Lens"}</h1>
            <p data-about-version>${info ? `Version ${info.version}` : nothing}</p>
            <p data-about-copyright>${info?.copyright || nothing}</p>
            <p data-about-error role="alert" ?hidden=${this.info.stage !== "failed"}>
              ${this.info.stage === "failed" ? `Unable to load app information: ${this.info.message}` : nothing}
            </p>
          </div>
        </header>
        <div class="documents">
          <label for="document">License documents</label>
          <select id="document" @change=${this.adoptNativeSelection}>
            <option value="license">LICENSE</option>
            <option value="notice">NOTICE</option>
          </select>
        </div>
        <div class="document-region">
          <div data-region-error="documents"></div>
          <lens-license-document
            class="document-text"
            role="region"
            aria-label="LICENSE"
            aria-busy="true"
            tabindex="0"
            .model=${this.documents}
            .document=${this.selectedDocument}
          ></lens-license-document>
        </div>
      </main>
    `;
  }
}
