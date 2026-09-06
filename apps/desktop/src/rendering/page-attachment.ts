import type { LitElement, ReactiveController, ReactiveControllerHost } from "lit";
import type { AttachmentState } from "./initial-state";
import { hydrateView } from "./hydrate-view";

export interface RegionModule {
  readonly name: string;
  readonly ready: () => boolean;
  readonly load: () => Promise<unknown>;
}
type ModuleState = "pending" | "loading" | "ready" | "failed";

/** Owns local DOM adoption; native resource controllers keep their existing authority. */
export class PageAttachment implements ReactiveController {
  stage: AttachmentState = "prerendered";
  private initialization: Promise<void> | undefined;
  private readonly modules = new Map<string, ModuleState>();

  constructor(
    private readonly host: HTMLElement & ReactiveControllerHost,
    private readonly view: () => LitElement,
    private readonly activate: () => void,
    private readonly regions: readonly RegionModule[],
  ) {
    host.addController(this);
    for (const region of regions) this.modules.set(region.name, "pending");
  }

  hostConnected(): void {
    if (this.stage === "active") this.host.requestUpdate();
  }

  initialize(): Promise<void> {
    return (this.initialization ??= this.attach());
  }

  private async attach(): Promise<void> {
    this.stage = "hydrating";
    try {
      await hydrateView(this.view());
      this.stage = "active";
      this.activate();
      this.host.requestUpdate();
      await this.host.updateComplete;
    } catch (error) {
      this.stage = "failed";
      throw error;
    }
  }

  hostUpdated(): void {
    if (this.stage !== "active" || !this.host.isConnected) return;
    // Property projection must commit before definitions upgrade the existing empty hosts.
    void this.view().updateComplete.then(() => {
      if (!this.host.isConnected) return;
      for (const region of this.regions) {
        if (region.ready() && this.modules.get(region.name) === "pending") void this.load(region);
      }
    });
  }

  private async load(region: RegionModule): Promise<void> {
    this.modules.set(region.name, "loading");
    const retry = this.errorRegion(region.name)?.querySelector("button");
    if (retry) retry.disabled = true;
    try {
      await region.load();
      this.modules.set(region.name, "ready");
      this.errorRegion(region.name)?.replaceChildren();
    } catch (error) {
      this.modules.set(region.name, "failed");
      const container = this.errorRegion(region.name);
      if (!container) throw error;
      const message = document.createElement("p");
      message.setAttribute("role", "alert");
      message.textContent = `Unable to load this region: ${String(error)}`;
      const retry = document.createElement("button");
      retry.type = "button";
      retry.textContent = "Retry";
      retry.addEventListener("click", () => {
        if (this.modules.get(region.name) === "failed") void this.load(region);
      });
      container.replaceChildren(message, retry);
    }
  }

  private errorRegion(name: string): HTMLElement | null {
    return (
      this.view().shadowRoot?.querySelector<HTMLElement>(`[data-region-error="${name}"]`) ?? null
    );
  }
}
