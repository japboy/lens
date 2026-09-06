import { snapshotStatus } from "../rendering/snapshot-status";
import { PageAttachment } from "../rendering/page-attachment";
import { ReactiveElement } from "lit";
import { customElement, state } from "lit/decorators.js";
import { AppSnapshotController } from "../application/app-snapshot-controller";
import { CommandController } from "../application/command-controller";
import type { CommandIdentity } from "../application/command-state";
import { settingsViewModel } from "../application/view-models";
import { tauriWebviewPort } from "../application/webview-port";
import { platformFromSearch } from "../presentation-context";
import { LensSettingsView } from "../components/lens-settings-view";
import type { SettingsIntent } from "../components/events";
import { AccessibilityPermissionController } from "../application/accessibility-permission-controller";
import { selectedAgent } from "../view-model";

@customElement("lens-settings-page")
export class SettingsPage extends ReactiveElement {
  private readonly port = tauriWebviewPort;
  private readonly snapshots = new AppSnapshotController(this, this.port);
  private readonly commands = new CommandController(this);
  private readonly platform = platformFromSearch(window.location.search);
  @state() private aboutOpenError = "";
  private readonly accessibility = new AccessibilityPermissionController(this, this.port);

  private readonly attachment = new PageAttachment(
    this,
    () => this.view,
    () => {
      this.view.activate();
    },
    [
      {
        name: "agent",
        ready: () => Boolean(this.snapshots.snapshot),
        load: () =>
          Promise.all([
            import("../components/lens-agent-settings"),
            import("../components/lens-agent-defaults"),
          ]),
      },
      {
        name: "prompt",
        ready: () => Boolean(this.snapshots.snapshot),
        load: () => import("../components/lens-prompt-settings"),
      },
    ],
  );

  initialize(): Promise<void> {
    return this.attachment.initialize();
  }

  private get view(): LensSettingsView {
    const view = this.querySelector("lens-settings-view");
    if (!(view instanceof LensSettingsView)) throw new Error("Missing settings view");
    return view;
  }

  protected createRenderRoot(): HTMLElement {
    return this;
  }
  connectedCallback(): void {
    super.connectedCallback();
    this.accessibility.setActive(true);
    this.addEventListener("lens-settings-intent", this.handleSettingsIntent);
  }
  disconnectedCallback(): void {
    super.disconnectedCallback();

    this.removeEventListener("lens-settings-intent", this.handleSettingsIntent);
  }
  protected update(changed: Map<PropertyKey, unknown>): void {
    super.update(changed);
    if (this.attachment.stage !== "active") return;
    const view = this.view;
    const snapshot = this.snapshots.snapshot;
    view.dataset.platform = this.platform;
    view.snapshotStatus = snapshotStatus(snapshot, this.snapshots.connection);
    view.model = snapshot
      ? settingsViewModel(
          this.platform,
          snapshot,
          this.accessibility.state,
          this.commands.state,
          this.snapshots.connection,
        )
      : undefined;
    view.aboutOpenError = this.aboutOpenError;
    view.permission = this.accessibility.state;
    view.commandPending = this.commands.state.stage === "pending";
  }

  private handleSettingsIntent = async (event: CustomEvent<SettingsIntent>): Promise<void> => {
    event.stopPropagation();
    if (this.attachment.stage !== "active") return;
    const intent = event.detail;
    if (intent.type === "open-about") {
      this.aboutOpenError = "";
      try {
        await this.port.showAbout();
      } catch (error) {
        this.aboutOpenError = `Unable to open About: ${String(error)}`;
      }
      return;
    }
    if (!this.snapshots.snapshot && intent.type !== "request-accessibility-permission") return;
    const identity: CommandIdentity = { scope: "settings", type: intent.type };
    switch (intent.type) {
      case "preview-agent-model": {
        const selectionId = this.snapshots.snapshot?.agent_selection.operation_id;
        if (!selectionId) return;
        await this.commands.run(identity, () =>
          this.port.previewAgentModel(selectionId, intent.configId, intent.value),
        );
        return;
      }
      case "save-agent-defaults": {
        const snapshot = this.snapshots.snapshot;
        const selection = snapshot?.agent_selection;
        if (!selection?.operation_id) return;
        const modeId = selection.config_options?.find((o) => o.category === "mode")?.id ?? "mode";
        const mode = intent.defaults.choices.find((c) => c.config_id === modeId)?.value;
        const elevated = Boolean(mode && mode !== selection.policy_default);
        const approved =
          !elevated ||
          (await this.port.confirmAction(
            `Use mode ${mode} for all new sessions of this Agent? It may allow changes or commands. Tool approval policies remain separate.`,
            "Save Shared Agent Mode",
          ));
        if (!approved) return;
        await this.commands.run(
          identity,
          () => this.port.setAgentDefaults(selection.operation_id!, intent.defaults, elevated),
          "Shared Agent settings saved.",
        );
        return;
      }
      case "select-agent":
        await this.commands.run(identity, () => this.port.setAgent(intent.agent));
        return;
      case "authenticate-agent-selection":
        await this.commands.run(identity, () =>
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
        await this.commands.run(identity, () => this.port.reauthenticateAgentSelection());
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
        await this.commands.run(identity, () => this.port.signOutAgentSelection());
        return;
      }
      case "save-agent-prompt-template":
        await this.commands.run(
          identity,
          () => this.port.setAgentPromptTemplate(intent.agentPromptTemplate),
          "Agent prompt template updated.",
        );
        return;
      case "reset-agent-prompt-template": {
        const approved = await this.port.confirmAction(
          "Reset every Agent Prompt section to the built-in defaults?",
          "Reset Agent Prompt Template",
        );
        if (!approved) return;
        await this.commands.run(
          identity,
          () => this.port.resetAgentPromptTemplate(),
          "Agent prompt template reset to the built-in defaults.",
        );
        return;
      }
      case "choose-directory":
        await this.chooseDirectory(identity);
        return;
      case "request-accessibility-permission":
        await this.commands.run(
          identity,
          () => this.accessibility.request(),
          "Allow Lens in System Settings, then check again.",
        );
        return;
    }
  };

  private async chooseDirectory(identity: CommandIdentity): Promise<void> {
    await this.commands.run(identity, async () => {
      const selected = await this.port.chooseDirectory(
        this.snapshots.snapshot?.config.working_directory,
      );
      if (!selected) return;
      await this.port.setWorkingDirectory(selected);
      return "Working directory updated.";
    });
  }
}
