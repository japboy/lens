import { LitElement, html, nothing, unsafeCSS, css } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { AboutDocuments, AboutInfo } from "../application/webview-port";
import {
  initialAboutState,
  documentKind,
  type Resource,
  type DocumentKind,
} from "../rendering/initial-state";
import aboutStyles from "../styles/about.css?inline";
import icon from "../../src-tauri/icons/128x128@2x.png";

@customElement("lens-about-view")
export class LensAboutView extends LitElement {
  static styles = [
    css`
      :host {
        display: block;
        height: 100%;
        min-height: 0;
      }
    `,
    unsafeCSS(aboutStyles),
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
