import { ReactiveElement } from "lit";
import { customElement } from "lit/decorators.js";
import { AppSnapshotController } from "../application/app-snapshot-controller";
import { CommandController } from "../application/command-controller";
import type { CommandIdentity } from "../application/command-state";
import { targetSelectionViewModel } from "../application/view-models";
import { tauriWebviewPort } from "../application/webview-port";
import { platformFromSearch } from "../presentation-context";
import { LensTargetSelectionView } from "../components/lens-target-selection-view";
import type { TargetSelectionIntent } from "../components/events";

@customElement("lens-target-selection-page")
export class TargetSelectionPage extends ReactiveElement {
  private readonly port = tauriWebviewPort;
  private readonly snapshots = new AppSnapshotController(this, this.port);
  private readonly commands = new CommandController(this);
  private readonly platform = platformFromSearch(window.location.search);

  protected createRenderRoot(): HTMLElement {
    return this;
  }
  connectedCallback(): void {
    super.connectedCallback();

    this.addEventListener("lens-target-selection-intent", this.handleTargetSelectionIntent);
  }
  disconnectedCallback(): void {
    super.disconnectedCallback();

    this.removeEventListener("lens-target-selection-intent", this.handleTargetSelectionIntent);
  }
  protected update(changed: Map<PropertyKey, unknown>): void {
    super.update(changed);
    const view = this.querySelector("lens-target-selection-view");
    if (!(view instanceof LensTargetSelectionView))
      throw new Error("Missing target-selection view");
    view.dataset.platform = this.platform;
    view.model = targetSelectionViewModel(
      this.platform,
      this.snapshots.snapshot,
      this.commands.state,
      this.snapshots.message(),
    );
  }

  private handleTargetSelectionIntent = async (
    event: CustomEvent<TargetSelectionIntent>,
  ): Promise<void> => {
    event.stopPropagation();
    const intent = event.detail;
    const identity: CommandIdentity = { scope: "target-selection", type: intent.type };
    const operationId = this.snapshots.snapshot?.lens.operation_id;
    if (!operationId) return;
    switch (intent.type) {
      case "add":
        await this.commands.run(identity, () => this.port.addLensTarget(operationId));
        return;
      case "remove":
        await this.commands.run(identity, () =>
          this.port.removeLensTarget(operationId, intent.targetId),
        );
        return;
      case "confirm":
        await this.commands.run(identity, () => this.port.confirmLensTargets(operationId));
        return;
    }
  };
}
