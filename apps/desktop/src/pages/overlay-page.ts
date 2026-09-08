import { snapshotStatus } from "../rendering/snapshot-status";
import { PageAttachment } from "../rendering/page-attachment";
import { ReactiveElement } from "lit";
import { customElement, state } from "lit/decorators.js";
import { AppSnapshotController } from "../application/app-snapshot-controller";
import { CommandController } from "../application/command-controller";
import { HtmlOutputController } from "../application/html-output-controller";
import type { CommandIdentity } from "../application/command-state";
import { overlayViewModel } from "../application/view-models";
import { tauriWebviewPort } from "../application/webview-port";
import { platformFromSearch } from "../presentation-context";
import { LensOverlayView } from "../components/lens-overlay-view";
import type { OverlayIntent } from "../components/events";
import type { InteractionSubmission } from "../application/view-models";

@customElement("lens-overlay-page")
export class OverlayPage extends ReactiveElement {
  private readonly port = tauriWebviewPort;
  private readonly snapshots = new AppSnapshotController(this, this.port);
  private readonly commands = new CommandController(this);
  private readonly htmlOutput = new HtmlOutputController(this, this.port);
  private readonly platform = platformFromSearch(window.location.search);
  @state() private interactionSubmission: InteractionSubmission | undefined;

  private readonly attachment = new PageAttachment(
    this,
    () => this.view,
    () => {
      this.view.active = true;
    },
    [
      {
        name: "output",
        ready: () => Boolean(this.snapshots.snapshot),
        load: () => import("../components/lens-agent-output"),
      },
      {
        name: "session",
        ready: () => Boolean(this.snapshots.snapshot),
        load: () => import("../components/lens-session-controls"),
      },
      {
        name: "source",
        ready: () => Boolean(this.snapshots.snapshot),
        load: () =>
          Promise.all([
            import("../components/lens-extraction-diagnostics"),
            import("../components/lens-media-gallery"),
          ]),
      },
    ],
  );

  initialize(): Promise<void> {
    return this.attachment.initialize();
  }

  private get view(): LensOverlayView {
    const view = this.querySelector("lens-overlay-view");
    if (!(view instanceof LensOverlayView)) throw new Error("Missing overlay view");
    return view;
  }

  protected createRenderRoot(): HTMLElement {
    return this;
  }
  connectedCallback(): void {
    super.connectedCallback();

    this.addEventListener("lens-overlay-intent", this.handleOverlayIntent);
  }
  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.interactionSubmission = undefined;
    this.removeEventListener("lens-overlay-intent", this.handleOverlayIntent);
  }
  protected update(changed: Map<PropertyKey, unknown>): void {
    super.update(changed);
    if (this.attachment.stage !== "active") return;
    const view = this.view;
    const snapshot = this.snapshots.snapshot;
    this.htmlOutput.synchronize(snapshot?.lens);
    view.htmlContent = this.htmlOutput.content;
    view.dataset.platform = this.platform;
    view.snapshotStatus = snapshotStatus(snapshot, this.snapshots.connection);
    view.model = snapshot
      ? {
          ...overlayViewModel(
            this.platform,
            snapshot,
            this.commands.state,
            this.snapshots.message(),
          ),
          interactionSubmission: this.interactionSubmission,
        }
      : undefined;
  }

  private handleOverlayIntent = async (event: CustomEvent<OverlayIntent>): Promise<void> => {
    event.stopPropagation();
    if (this.attachment.stage !== "active") return;
    const intent = event.detail;
    const identity: CommandIdentity = { scope: "overlay", type: intent.type };
    const lens = this.snapshots.snapshot?.lens;
    if (intent.type === "close") {
      const operationId = lens?.operation_id;
      await this.commands.run(identity, async () => {
        if (operationId) await this.port.stopLens(operationId);
        await this.port.closeCurrentWindow();
      });
      return;
    }
    if (!this.snapshots.snapshot) return;
    switch (intent.type) {
      case "set-session-option": {
        if (!lens?.operation_id) return;
        await this.commands.run(identity, () =>
          this.port.setSessionOption(
            lens.operation_id!,
            intent.instanceId,
            intent.revision,
            intent.configId,
            intent.value,
          ),
        );
        return;
      }
      case "respond-interaction": {
        if (!lens?.operation_id) return;
        if (
          this.interactionSubmission?.instanceId === intent.instanceId &&
          this.interactionSubmission.interactionId === intent.interactionId &&
          this.interactionSubmission.stage !== "failed"
        )
          return;
        const submission: InteractionSubmission = {
          instanceId: intent.instanceId,
          interactionId: intent.interactionId,
          stage: "sending",
        };
        this.interactionSubmission = submission;
        try {
          await this.port.respondAgentInteraction(
            lens.operation_id,
            intent.instanceId,
            intent.interactionId,
            intent.response,
          );
          if (this.interactionSubmission === submission)
            this.interactionSubmission = { ...submission, stage: "sent" };
        } catch (error) {
          if (this.interactionSubmission === submission)
            this.interactionSubmission = { ...submission, stage: "failed", message: String(error) };
        }
        return;
      }
      case "authenticate": {
        const operationId = lens?.operation_id;
        if (!operationId) return;
        await this.commands.run(identity, () =>
          this.port.authenticateAgent(operationId, intent.methodId),
        );
        return;
      }
      case "retry": {
        const operationId = lens?.operation_id;
        if (!operationId) return;
        await this.commands.run(identity, () => this.port.retryLensTransform(operationId));
        return;
      }
      case "cancel": {
        const operationId = lens?.operation_id;
        const runId = lens?.agent?.run_id;
        if (!operationId || !runId) return;
        await this.commands.run(identity, () => this.port.cancelAgent(operationId, runId));
        return;
      }
      case "pause": {
        const operationId = lens?.operation_id;
        if (!operationId) return;
        await this.commands.run(identity, () => this.port.pauseLens(operationId));
        return;
      }
      case "resume": {
        const operationId = lens?.operation_id;
        if (!operationId) return;
        await this.commands.run(identity, () => this.port.resumeLens(operationId));
        return;
      }
      case "open-external-url":
        await this.commands.run(identity, () => this.port.openExternalUrl(intent.url));
        return;
      case "report-error":
        this.commands.reportFailure(identity, intent.message);
        return;
    }
  };
}
