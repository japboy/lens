import "./lens-select";
import { controlStyles } from "../styles/component-styles";
import type { LensSelect } from "./lens-select";
import { LitElement, html, nothing, css } from "lit";
import { ABOUT_INTENT_EVENT, dispatchComponentEvent, type AboutIntent } from "./events";
import { customElement, property, state } from "lit/decorators.js";
import type { AboutDocuments, AboutInfo, ReleaseAvailability } from "../application/webview-port";
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
    controlStyles,
    css`
      :host {
        display: block;
        height: 100%;
        min-height: 0;
        --about-link-color: #1265be;
        --about-update-border: #d8dbe1;
        --about-update-background: #f5f6f8;
        --about-update-available-border: #b8cde7;
        --about-update-available-background: #f0f6fd;
        --about-update-secondary: #5d6570;
      }
      @media (prefers-color-scheme: dark) {
        :host {
          --about-link-color: #76b8ff;
          --about-update-border: #4b4d56;
          --about-update-background: #2c2e34;
          --about-update-available-border: #416487;
          --about-update-available-background: #233346;
          --about-update-secondary: #b7bdc8;
        }
      }

      main {
        box-sizing: border-box;
        height: 100%;
        padding: 24px;
        display: grid;
        grid-template-rows: auto auto auto minmax(0, 1fr);
        gap: 18px;
      }
      header {
        display: grid;
        grid-template-columns: 96px minmax(0, 1fr);
        align-items: center;
        gap: 20px;
        justify-self: center;
        width: min(100%, 390px);
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
      a {
        color: var(--about-link-color);
        text-decoration: none;
      }
      a:hover {
        text-decoration: underline;
      }
      a:focus-visible,
      button:focus-visible {
        outline: 2px solid Highlight;
        outline-offset: 2px;
      }
      .repository-link {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        margin-top: 4px;
      }
      .github-mark {
        width: 16px;
        height: 16px;
        flex: none;
        fill: currentColor;
      }
      .external-icon {
        width: 12px;
        height: 12px;
        flex: none;
      }
      .update {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 14px;
        min-height: 58px;
        padding: 11px 14px;
        border: 1px solid var(--about-update-border);
        border-radius: 7px;
        background: var(--about-update-background);
      }
      .update[data-state="available"] {
        border-color: var(--about-update-available-border);
        background: var(--about-update-available-background);
      }
      .update-copy {
        min-width: 0;
      }
      .update strong {
        display: block;
        font-size: 13px;
        font-weight: 600;
        line-height: 1.3;
      }
      .update small {
        display: block;
        margin-top: 3px;
        color: var(--about-update-secondary);
        line-height: 1.3;
      }
      .update a {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        flex: none;
        white-space: nowrap;
        font-weight: 500;
      }
      .update button {
        flex: none;
      }
      .documents {
        display: flex;
        align-items: center;
        gap: 12px;
      }
      .documents lens-select {
        font: inherit;
      }
      .document-region {
        display: grid;
        grid-template-rows: auto minmax(0, 1fr);
        min-height: 0;
      }
      [data-region-error]:empty {
        display: none;
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
  @property({ attribute: false }) releaseAvailability: ReleaseAvailability = {
    revision: -1,
    stage: "idle",
  };
  @property({ attribute: false }) retryAvailable = true;
  @state() private selectedDocument: DocumentKind = initialAboutState().document;

  private emitIntent(type: AboutIntent["type"]): void {
    dispatchComponentEvent<AboutIntent>(this, ABOUT_INTENT_EVENT, { type });
  }

  private openRepository(event: MouseEvent): void {
    event.preventDefault();
    this.emitIntent("open-repository");
  }

  private openRelease(event: MouseEvent): void {
    event.preventDefault();
    this.emitIntent("open-release");
  }

  private updateCopy(currentVersion: string | undefined): { title: string; description: string } {
    switch (this.releaseAvailability.stage) {
      case "idle":
      case "checking":
        return {
          title: "Checking for updates…",
          description: currentVersion
            ? `The current version is ${currentVersion}.`
            : "Checking the latest release on GitHub.",
        };
      case "current":
        return {
          title: "No newer release was found",
          description: currentVersion
            ? `GitHub's latest release is not newer than version ${currentVersion}.`
            : "GitHub's latest release is not newer than the installed version.",
        };
      case "available":
        return {
          title: `Version ${this.releaseAvailability.version} is available`,
          description: "You can download the latest release from GitHub.",
        };
      case "failed":
        return {
          title: "Unable to check for updates",
          description: this.retryAvailable
            ? "The update check could not be completed. Please try again."
            : "Update checks are temporarily unavailable. Try again shortly.",
        };
    }
  }

  adoptDocumentSelection(): void {
    const select = this.shadowRoot?.querySelector<LensSelect>("lens-select");
    if (!select) throw new Error("Missing About document select");
    this.selectedDocument = documentKind(select.value);
  }

  protected render() {
    const info = this.info.stage === "ready" ? this.info.value : undefined;
    const updateCopy = this.updateCopy(info?.version);
    const availability = this.releaseAvailability;
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
            <a
              class="repository-link"
              href="https://github.com/japboy/lens"
              @click=${this.openRepository}
            >
              <!-- Font Awesome Free 7.3.1 brands/github.svg; CC BY 4.0. -->
              <svg class="github-mark" viewBox="0 0 512 512" aria-hidden="true">
                <path
                  d="M216.5 362.5c-66-8-112.5-55.5-112.5-117 0-25 9-52 24-70-6.5-16.5-5.5-51.5 2-66 20-2.5 47 8 63 22.5 19-6 39-9 63.5-9s44.5 3 62.5 8.5c15.5-14 43-24.5 63-22 7 13.5 8 48.5 1.5 65.5 16 19 24.5 44.5 24.5 70.5 0 61.5-46.5 108-113.5 116.5 17 11 28.5 35 28.5 62.5l0 52C323 491.5 335.5 500 350.5 494 441 459.5 512 369 512 257 512 115.5 397 0 255.5 0S0 115.5 0 257c0 111 70.5 203 165.5 237.5 13.5 5 26.5-4 26.5-17.5l0-40c-7 3-16 5-24 5-33 0-52.5-18-66.5-51.5-5.5-13.5-11.5-21.5-23-23-6-.5-8-3-8-6 0-6 10-10.5 20-10.5 14.5 0 27 9 40 27.5 10 14.5 20.5 21 33 21s20.5-4.5 32-16c8.5-8.5 15-16 21-21z"
                />
              </svg>
              japboy/lens
              <svg
                class="external-icon"
                viewBox="0 0 16 16"
                fill="none"
                stroke="currentColor"
                stroke-width="1.6"
                aria-hidden="true"
              >
                <path d="M6 3h7v7M13 3 5 11" />
                <path d="M12 9v4H3V4h4" />
              </svg>
            </a>
          </div>
        </header>
        <section class="update" data-state=${availability.stage} role="status" aria-live="polite">
          <div class="update-copy">
            <strong>${updateCopy.title}</strong>
            <small>${updateCopy.description}</small>
          </div>
          ${
            availability.stage === "available"
              ? html`<a
                  class="release-link"
                  href=${availability.release_url}
                  @click=${this.openRelease}
                >
                  View release
                  <svg
                    class="external-icon"
                    viewBox="0 0 16 16"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    aria-hidden="true"
                  >
                    <path d="M6 3h7v7M13 3 5 11" />
                    <path d="M12 9v4H3V4h4" />
                  </svg>
                </a>`
              : nothing
          }
          ${
            availability.stage === "failed"
              ? html`<button
                  type="button"
                  ?disabled=${!this.retryAvailable}
                  @click=${() => this.emitIntent("retry-update-check")}
                >
                  Retry
                </button>`
              : nothing
          }
        </section>
        <div class="documents">
          <label for="document">License documents</label>
          <lens-select
            id="document"
            label="License documents"
            .value=${this.selectedDocument}
            .options=${[
              { value: "license", label: "LICENSE" },
              { value: "notice", label: "NOTICE" },
            ]}
            @change=${this.adoptDocumentSelection}
          ></lens-select>
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
