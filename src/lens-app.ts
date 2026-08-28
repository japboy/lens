import { LitElement, css, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { AccessibilityPermissionController } from "./application/accessibility-permission-controller";
import { AppSnapshotController } from "./application/app-snapshot-controller";
import {
  canStartCommand,
  IDLE_COMMAND_STATE,
  type CommandIdentity,
  type CommandState,
} from "./application/command-state";
import {
  overlayViewModel,
  settingsViewModel,
  targetSelectionViewModel,
} from "./application/view-models";
import { tauriWebviewPort } from "./application/webview-port";
import "./components/lens-overlay-view";
import "./components/lens-settings-view";
import "./components/lens-target-selection-view";
import type { OverlayIntent, SettingsIntent, TargetSelectionIntent } from "./components/events";
import type { PresentationContext } from "./presentation-context";
import { selectedAgent } from "./view-model";

@customElement("lens-app")
export class LensApp extends LitElement {
  static styles = css`
    :host {
      display: block;
      width: 100%;
      height: 100%;
      min-width: 0;
      min-height: 0;
    }
  `;

  @property({ attribute: false })
  context: PresentationContext | undefined;

  @state()
  private command: CommandState = IDLE_COMMAND_STATE;

  private commandGeneration = 0;

  private readonly port = tauriWebviewPort;
  private readonly snapshots = new AppSnapshotController(this, this.port);
  private readonly accessibility = new AccessibilityPermissionController(this, this.port);

  protected willUpdate(changed: PropertyValues<this>): void {
    if (changed.has("context")) {
      this.accessibility.setActive(this.context?.view === "settings");
    }
  }

  protected render() {
    const context = this.context;
    if (!context) return nothing;
    const snapshot = this.snapshots.snapshot;
    const connectionMessage = this.snapshots.message();
    switch (context.view) {
      case "settings":
        return html`<lens-settings-view
          data-platform=${context.platform}
          .model=${settingsViewModel(
            context.platform,
            snapshot,
            this.accessibility.state,
            this.command,
            connectionMessage,
          )}
          @lens-settings-intent=${this.handleSettingsIntent}
        ></lens-settings-view>`;
      case "target-selection":
        return html`<lens-target-selection-view
          data-platform=${context.platform}
          .model=${targetSelectionViewModel(
            context.platform,
            snapshot,
            this.command,
            connectionMessage,
          )}
          @lens-target-selection-intent=${this.handleTargetSelectionIntent}
        ></lens-target-selection-view>`;
      case "overlay":
        return html`<lens-overlay-view
          data-platform=${context.platform}
          .model=${overlayViewModel(context.platform, snapshot, this.command, connectionMessage)}
          @lens-overlay-intent=${this.handleOverlayIntent}
        ></lens-overlay-view>`;
    }
  }

  private handleSettingsIntent = async (event: CustomEvent<SettingsIntent>): Promise<void> => {
    event.stopPropagation();
    const intent = event.detail;
    const identity: CommandIdentity = { scope: "settings", type: intent.type };
    switch (intent.type) {
      case "select-agent":
        await this.runCommand(identity, () => this.port.setAgent(intent.agent));
        return;
      case "authenticate-agent-selection":
        await this.runCommand(identity, () =>
          this.port.authenticateAgentSelection(intent.methodId),
        );
        return;
      case "reauthenticate-agent-selection": {
        const agent = this.snapshots.snapshot
          ? selectedAgent(this.snapshots.snapshot.agent_selection)
          : undefined;
        if (!agent) return;
        const label = agent === "claude" ? "Claude" : "Codex";
        const approved = await this.port.confirmAction(
          `Reauthentication signs out of ${label} first. Continue?`,
          `Reauthenticate ${label}`,
        );
        if (!approved) return;
        await this.runCommand(identity, () => this.port.reauthenticateAgentSelection());
        return;
      }
      case "sign-out-agent-selection": {
        const agent = this.snapshots.snapshot
          ? selectedAgent(this.snapshots.snapshot.agent_selection)
          : undefined;
        if (!agent) return;
        const label = agent === "claude" ? "Claude" : "Codex";
        const approved = await this.port.confirmAction(
          `Sign out of ${label}? This changes the authentication used by its existing CLI.`,
          `Sign Out of ${label}`,
        );
        if (!approved) return;
        await this.runCommand(identity, () => this.port.signOutAgentSelection());
        return;
      }
      case "save-response-prompt":
        await this.runCommand(
          identity,
          () => this.port.setResponsePrompt(intent.responsePrompt),
          "Agent prompt updated.",
        );
        return;
      case "reset-response-prompt": {
        const approved = await this.port.confirmAction(
          "Reset the Agent Prompt to the built-in default?",
          "Reset Agent Prompt",
        );
        if (!approved) return;
        await this.runCommand(
          identity,
          () => this.port.resetResponsePrompt(),
          "Agent prompt reset to the built-in default.",
        );
        return;
      }
      case "choose-directory":
        await this.chooseDirectory(identity);
        return;
      case "request-accessibility-permission":
        await this.runCommand(
          identity,
          () => this.accessibility.request(),
          "Allow Lens in System Settings, then check again.",
        );
        return;
    }
  };

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
        await this.runCommand(identity, () => this.port.addLensTarget(operationId));
        return;
      case "remove":
        await this.runCommand(identity, () =>
          this.port.removeLensTarget(operationId, intent.targetId),
        );
        return;
      case "confirm":
        await this.runCommand(identity, () => this.port.confirmLensTargets(operationId));
        return;
    }
  };

  private handleOverlayIntent = async (event: CustomEvent<OverlayIntent>): Promise<void> => {
    event.stopPropagation();
    const intent = event.detail;
    const identity: CommandIdentity = { scope: "overlay", type: intent.type };
    const lens = this.snapshots.snapshot?.lens;
    switch (intent.type) {
      case "authenticate": {
        const operationId = lens?.operation_id;
        if (!operationId) return;
        await this.runCommand(identity, () =>
          this.port.authenticateAgent(operationId, intent.methodId),
        );
        return;
      }
      case "transform": {
        const operationId = lens?.operation_id;
        if (!operationId) return;
        await this.runCommand(identity, () => this.port.transformLens(operationId));
        return;
      }
      case "cancel": {
        const operationId = lens?.operation_id;
        const runId = lens?.agent?.run_id;
        if (!operationId || !runId) return;
        await this.runCommand(identity, () => this.port.cancelAgent(operationId, runId));
        return;
      }
      case "close":
        await this.runCommand(identity, () => this.port.closeCurrentWindow());
        return;
      case "open-external-url":
        await this.runCommand(identity, () => this.port.openExternalUrl(intent.url));
        return;
      case "report-error":
        this.commandGeneration += 1;
        this.command = { stage: "failed", command: identity, message: intent.message };
        return;
    }
  };

  private async chooseDirectory(identity: CommandIdentity): Promise<void> {
    if (this.command.stage === "pending") return;
    this.command = { stage: "pending", command: identity };
    try {
      const selected = await this.port.chooseDirectory(
        this.snapshots.snapshot?.config.working_directory,
      );
      if (!selected) {
        this.command = IDLE_COMMAND_STATE;
        return;
      }
      await this.port.setWorkingDirectory(selected);
      this.command = {
        stage: "succeeded",
        command: identity,
        message: "Working directory updated.",
      };
    } catch (error) {
      this.command = { stage: "failed", command: identity, message: String(error) };
    }
  }

  private async runCommand(
    command: CommandIdentity,
    action: () => Promise<void>,
    successMessage = "",
  ): Promise<void> {
    if (!canStartCommand(this.command, command)) return;
    const generation = ++this.commandGeneration;
    this.command = { stage: "pending", command };
    try {
      await action();
      if (generation !== this.commandGeneration) return;
      this.command = successMessage
        ? { stage: "succeeded", command, message: successMessage }
        : IDLE_COMMAND_STATE;
    } catch (error) {
      if (generation !== this.commandGeneration) return;
      this.command = { stage: "failed", command, message: String(error) };
    }
  }
}
