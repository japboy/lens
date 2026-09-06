import { ReactiveElement } from "lit";
import { customElement, state } from "lit/decorators.js";
import { tauriWebviewPort } from "../application/webview-port";
import { LensAboutView } from "../components/lens-about-view";
import { initialAboutState } from "../rendering/initial-state";
import { PageAttachment } from "../rendering/page-attachment";

@customElement("lens-about-page")
export class AboutPage extends ReactiveElement {
  @state() private info = initialAboutState().info;
  @state() private documents = initialAboutState().documents;
  private readonly attachment = new PageAttachment(
    this,
    () => this.view,
    () => {
      this.view.adoptNativeSelection();
      if (this.documents.stage !== "ready") void this.loadDocuments(this.generation);
    },
    [
      {
        name: "documents",
        ready: () => true,
        load: () => import("../components/lens-license-document"),
      },
    ],
  );
  private generation = 0;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  connectedCallback(): void {
    super.connectedCallback();
    const generation = ++this.generation;
    if (this.info.stage !== "ready") void this.loadInfo(generation);
    if (this.attachment.stage === "active" && this.documents.stage !== "ready")
      void this.loadDocuments(generation);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.generation += 1;
  }

  initialize(): Promise<void> {
    return this.attachment.initialize();
  }

  private get view(): LensAboutView {
    const view = this.querySelector("lens-about-view");
    if (!(view instanceof LensAboutView)) throw new Error("Missing About view");
    return view;
  }

  protected update(changed: Map<PropertyKey, unknown>): void {
    super.update(changed);
    if (this.attachment.stage !== "active") return;
    this.view.info = this.info;
    this.view.documents = this.documents;
  }

  private async loadInfo(generation: number): Promise<void> {
    if (!this.isConnected || generation !== this.generation) return;
    try {
      const value = await tauriWebviewPort.getAboutInfo();
      if (generation === this.generation) this.info = { stage: "ready", value };
    } catch (error) {
      if (generation === this.generation) this.info = { stage: "failed", message: String(error) };
    }
  }

  private async loadDocuments(generation: number): Promise<void> {
    if (!this.isConnected || generation !== this.generation) return;
    try {
      const value = await tauriWebviewPort.getAboutDocuments();
      if (generation === this.generation) this.documents = { stage: "ready", value };
    } catch (error) {
      if (generation === this.generation)
        this.documents = { stage: "failed", message: String(error) };
    }
  }
}
