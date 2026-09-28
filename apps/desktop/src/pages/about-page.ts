import { ReactiveElement } from "lit";
import { customElement, state } from "lit/decorators.js";
import {
  tauriWebviewPort,
  type ReleaseAvailability,
  type Unlisten,
} from "../application/webview-port";
import type { AboutIntent } from "../components/events";
import { LensAboutView } from "../components/lens-about-view";
import { initialAboutState } from "../rendering/initial-state";
import { PageAttachment } from "../rendering/page-attachment";

@customElement("lens-about-page")
export class AboutPage extends ReactiveElement {
  @state() private info = initialAboutState().info;
  @state() private documents = initialAboutState().documents;
  @state() private releaseAvailability: ReleaseAvailability = { revision: -1, stage: "idle" };
  @state() private releaseReadFailed = false;
  @state() private retryClock = Date.now();
  private releaseUnlisten: Unlisten | undefined;
  private retryTimer: ReturnType<typeof setTimeout> | undefined;
  private releasePollTimer: ReturnType<typeof setTimeout> | undefined;
  private releasePollAttempt = 0;
  private readonly attachment = new PageAttachment(
    this,
    () => this.view,
    () => {
      this.view.adoptDocumentSelection();
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
    this.addEventListener("lens-about-intent", this.handleAboutIntent);
    this.refreshRetryClock();
    void this.observeReleaseAvailability(generation);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.generation += 1;
    this.releaseUnlisten?.();
    this.releaseUnlisten = undefined;
    clearTimeout(this.retryTimer);
    this.retryTimer = undefined;
    clearTimeout(this.releasePollTimer);
    this.releasePollTimer = undefined;
    this.releasePollAttempt = 0;
    this.removeEventListener("lens-about-intent", this.handleAboutIntent);
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
    this.view.releaseAvailability = this.releaseReadFailed
      ? { revision: this.releaseAvailability.revision, stage: "failed" }
      : this.releaseAvailability;
    this.view.retryAvailable =
      this.releaseAvailability.stage !== "failed" ||
      !this.releaseAvailability.retry_after_epoch_ms ||
      this.releaseAvailability.retry_after_epoch_ms <= this.retryClock;
  }

  private acceptReleaseAvailability(availability: ReleaseAvailability): void {
    if (availability.revision < this.releaseAvailability.revision) return;
    if (availability.revision === this.releaseAvailability.revision && !this.releaseReadFailed)
      return;
    this.releaseReadFailed = false;
    this.releaseAvailability = availability;
    this.refreshRetryClock();
  }

  private refreshRetryClock(): void {
    clearTimeout(this.retryTimer);
    this.retryTimer = undefined;
    this.retryClock = Date.now();
    const availability = this.releaseAvailability;
    if (availability.stage !== "failed" || !availability.retry_after_epoch_ms) return;
    const remaining = availability.retry_after_epoch_ms - this.retryClock;
    if (remaining > 0) {
      this.retryTimer = setTimeout(
        () => this.refreshRetryClock(),
        Math.min(remaining, 2_147_483_647),
      );
    }
  }

  private async subscribeToReleaseAvailability(generation: number): Promise<void> {
    if (this.releaseUnlisten) return;
    try {
      const unlisten = await tauriWebviewPort.subscribeToReleaseAvailability((availability) => {
        if (generation === this.generation) this.acceptReleaseAvailability(availability);
      });
      if (generation !== this.generation) {
        unlisten();
        return;
      }
      this.releaseUnlisten = unlisten;
      clearTimeout(this.releasePollTimer);
      this.releasePollTimer = undefined;
    } catch {
      // Snapshot reads remain available when event registration fails.
    }
  }

  private scheduleReleasePoll(generation: number): void {
    if (this.releaseUnlisten || this.releasePollTimer) return;
    const pending =
      this.releaseAvailability.stage === "idle" || this.releaseAvailability.stage === "checking";
    const delay =
      pending && this.releasePollAttempt < 3
        ? this.releasePollAttempt === 0
          ? 12_000
          : 2_000
        : 5 * 60_000;
    this.releasePollTimer = setTimeout(async () => {
      this.releasePollTimer = undefined;
      if (generation !== this.generation) return;
      if (pending) this.releasePollAttempt += 1;
      try {
        const availability = await tauriWebviewPort.getReleaseAvailability();
        if (generation !== this.generation) return;
        this.acceptReleaseAvailability(availability);
      } catch {
        if (generation !== this.generation) return;
        this.releaseReadFailed = true;
      }
      const stillPending =
        this.releaseAvailability.stage === "idle" || this.releaseAvailability.stage === "checking";
      if (!stillPending) this.releasePollAttempt = 0;
      else if (this.releasePollAttempt >= 3) this.releaseReadFailed = true;
      this.scheduleReleasePoll(generation);
    }, delay);
  }

  private async observeReleaseAvailability(generation: number): Promise<void> {
    await this.subscribeToReleaseAvailability(generation);
    if (generation !== this.generation) return;
    try {
      const availability = await tauriWebviewPort.getReleaseAvailability();
      if (generation !== this.generation) return;
      this.acceptReleaseAvailability(availability);
      this.scheduleReleasePoll(generation);
    } catch {
      if (generation !== this.generation) return;
      this.releaseReadFailed = true;
      this.scheduleReleasePoll(generation);
    }
  }

  private async retryReleaseAvailability(generation: number): Promise<void> {
    await this.subscribeToReleaseAvailability(generation);
    if (generation !== this.generation) return;
    clearTimeout(this.releasePollTimer);
    this.releasePollTimer = undefined;
    this.releasePollAttempt = 0;
    try {
      const availability = await tauriWebviewPort.retryReleaseAvailabilityCheck();
      if (generation !== this.generation) return;
      this.acceptReleaseAvailability(availability);
      this.scheduleReleasePoll(generation);
    } catch {
      if (generation !== this.generation) return;
      this.releaseReadFailed = true;
      this.scheduleReleasePoll(generation);
    }
  }

  private readonly handleAboutIntent = (event: CustomEvent<AboutIntent>): void => {
    switch (event.detail.type) {
      case "open-repository":
        void tauriWebviewPort.openExternalUrl("https://github.com/japboy/lens");
        return;
      case "open-release":
        if (this.releaseAvailability.stage === "available")
          void tauriWebviewPort.openExternalUrl(this.releaseAvailability.release_url);
        return;
      case "retry-update-check":
        if (!this.releaseReadFailed && this.releaseAvailability.stage !== "failed") return;
        if (
          this.releaseAvailability.stage === "failed" &&
          this.releaseAvailability.retry_after_epoch_ms &&
          this.releaseAvailability.retry_after_epoch_ms > Date.now()
        )
          return;
        void this.retryReleaseAvailability(this.generation);
        return;
    }
  };

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
