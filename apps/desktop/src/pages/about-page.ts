import { ReactiveElement } from "lit";
import { customElement, state } from "lit/decorators.js";
import { tauriWebviewPort } from "../application/webview-port";
import type { AboutInfo } from "../application/webview-port";
import { LensLicenseDocument, type AboutResource } from "../components/lens-license-document";

@customElement("lens-about-page")
export class AboutPage extends ReactiveElement {
  @state() private info: AboutResource<AboutInfo> = { stage: "loading" };
  private generation = 0;
  protected createRenderRoot(): HTMLElement {
    return this;
  }

  connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener("change", this.selectDocument);
    void this.load(++this.generation);
  }
  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.generation += 1;
    this.removeEventListener("change", this.selectDocument);
  }
  protected update(changed: Map<PropertyKey, unknown>): void {
    super.update(changed);
    if (this.info.stage === "ready") {
      this.field("name").textContent = this.info.value.name;
      this.field("version").textContent = `Version ${this.info.value.version}`;
      this.field("copyright").textContent = this.info.value.copyright;
    }
    const error = this.field("error");
    error.hidden = this.info.stage !== "failed";
    error.textContent =
      this.info.stage === "failed" ? `Unable to load app information: ${this.info.message}` : "";
  }
  private field(name: string): HTMLElement {
    const field = this.querySelector<HTMLElement>(`[data-about-${name}]`);
    if (!field) throw new Error(`Missing About ${name} field`);
    return field;
  }
  private get documentView(): LensLicenseDocument {
    const view = this.querySelector("lens-license-document");
    if (!(view instanceof LensLicenseDocument)) throw new Error("Missing license document region");
    return view;
  }
  private selectDocument = (): void => {
    const value = this.querySelector<HTMLSelectElement>("select")?.value;
    if (value === "license" || value === "notice") this.documentView.document = value;
  };
  private async load(generation: number): Promise<void> {
    await this.updateComplete;
    if (generation !== this.generation) return;
    this.selectDocument();
    try {
      const value = await tauriWebviewPort.getAboutInfo();
      if (generation !== this.generation) return;
      this.info = { stage: "ready", value };
    } catch (error) {
      if (generation !== this.generation) return;
      this.info = { stage: "failed", message: String(error) };
    }
    await this.updateComplete;
    if (generation !== this.generation) return;
    try {
      const value = await tauriWebviewPort.getAboutDocuments();
      if (generation === this.generation) this.documentView.model = { stage: "ready", value };
    } catch (error) {
      if (generation === this.generation)
        this.documentView.model = { stage: "failed", message: String(error) };
    }
  }
}
