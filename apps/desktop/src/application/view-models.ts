import type { DesktopPlatform } from "ui/contracts/context";
import type { SettingsDestination } from "ui/presentation/agent-prompt-template";
import type { AppSnapshot } from "../types";
import { STAGE_LABEL } from "ui/presentation/view-model";
import type { AccessibilityPermissionState } from "ui/contracts/resource-state";
import { snapshotConnectionMessage } from "./app-snapshot-controller";
import { type SnapshotConnectionState } from "ui/contracts/resource-state";
import {
  commandMessage,
  isPendingCommand,
  type CommandIdentity,
  type CommandState,
} from "./command-state";

import type {
  SettingsViewModel,
  TargetSelectionViewModel,
  OverlayViewModel,
  SettingsFeedback,
} from "ui/contracts/view-models";

function presentationMessage(command: CommandState, connectionMessage: string): string {
  return commandMessage(command) || connectionMessage;
}

type SettingsCommandType = Extract<CommandIdentity, { scope: "settings" }>["type"];

function settingsFeedbackTarget(command: SettingsCommandType): SettingsDestination {
  switch (command) {
    case "preview-agent-model":
    case "save-agent-defaults":
      return "session-defaults";
    case "select-agent":
    case "update-managed-agent":
    case "choose-external-executable":
    case "set-mcp-apps-servers":
    case "reset-mcp-presets":
    case "save-external-agent":
    case "delete-external-agent":
    case "reset-agent-presets":
    case "authenticate-agent-selection":
    case "reauthenticate-agent-selection":
    case "sign-out-agent-selection":
    case "choose-directory":
      return "connection";
    case "request-accessibility-permission":
    case "open-screen-recording-settings":
      return "privacy-security";
    case "update-prompt-presets":
    case "save-agent-prompt-template":
    case "reset-agent-prompt-template":
      return "prompt-presets";
  }
}

function connectionFeedback(connection: SnapshotConnectionState): SettingsFeedback {
  switch (connection.stage) {
    case "subscribing":
    case "loading":
      return {
        stage: "status",
        target: "application",
        message: snapshotConnectionMessage(connection),
      };
    case "ready":
      return { stage: "none" };
    case "failed":
      return {
        stage: "error",
        target: "application",
        message: snapshotConnectionMessage(connection),
      };
  }
}

function settingsPendingMessage(command: SettingsCommandType): string {
  switch (command) {
    case "preview-agent-model":
      return "Loading model options…";
    case "save-agent-defaults":
      return "Saving session defaults…";
    case "select-agent":
      return "Verifying connection…";
    case "update-managed-agent":
      return "Updating Agent…";
    case "choose-external-executable":
      return "Choosing executable…";
    case "set-mcp-apps-servers":
      return "Saving MCP presets…";
    case "reset-mcp-presets":
      return "Resetting MCP presets…";
    case "save-external-agent":
      return "Saving Agent preset…";
    case "delete-external-agent":
      return "Deleting Agent preset…";
    case "reset-agent-presets":
      return "Resetting Agent presets…";
    case "authenticate-agent-selection":
      return "Authenticating Agent…";
    case "reauthenticate-agent-selection":
      return "Reauthenticating Agent…";
    case "sign-out-agent-selection":
      return "Signing out…";
    case "choose-directory":
      return "Choosing working directory…";
    case "request-accessibility-permission":
      return "Requesting Accessibility permission…";
    case "open-screen-recording-settings":
      return "Opening Screen Recording settings…";
    case "update-prompt-presets":
      return "Saving prompt presets…";
    case "save-agent-prompt-template":
      return "Saving prompt template…";
    case "reset-agent-prompt-template":
      return "Resetting prompt template…";
  }
}

export function settingsFeedback(
  command: CommandState,
  connection?: SnapshotConnectionState,
): SettingsFeedback {
  if (command.stage === "pending" && command.command.scope === "settings") {
    return {
      stage: "status",
      target: settingsFeedbackTarget(command.command.type),
      message: settingsPendingMessage(command.command.type),
    };
  }
  if (
    (command.stage === "succeeded" || command.stage === "failed") &&
    command.command.scope === "settings"
  ) {
    return {
      stage: command.stage === "failed" ? "error" : "status",
      target: settingsFeedbackTarget(command.command.type),
      message: command.message,
    };
  }
  return connection ? connectionFeedback(connection) : { stage: "none" };
}

export function settingsViewModel(
  platform: DesktopPlatform,
  snapshot: AppSnapshot,
  permission: AccessibilityPermissionState,
  command: CommandState,
  connection: SnapshotConnectionState,
  commands: CommandState[] = [command],
): SettingsViewModel {
  const lens = snapshot.lens;
  const pendingDestinations = new Set<SettingsDestination>();
  const feedbackByDestination: Partial<Record<SettingsDestination, SettingsFeedback>> = {};
  for (const state of commands) {
    if (state.stage === "idle" || state.command.scope !== "settings") continue;
    const destination = settingsFeedbackTarget(state.command.type);
    feedbackByDestination[destination] = settingsFeedback(state);
    if (state.stage === "pending") {
      pendingDestinations.add(destination);
      if (destination === "connection" || destination === "session-defaults") {
        pendingDestinations.add("connection");
        pendingDestinations.add("session-defaults");
      }
    }
  }
  const updateCommand = commands.find((state) =>
    isPendingCommand(state, "settings", "update-managed-agent"),
  );
  const promptCommand =
    commands.find(
      (state) =>
        state.stage !== "idle" &&
        state.command.scope === "settings" &&
        settingsFeedbackTarget(state.command.type) === "prompt-presets",
    ) ?? command;
  return {
    platform,
    config: snapshot.config,
    agentSelection: snapshot.agent_selection,
    agentRuntime: snapshot.agent_runtime,
    updatePending: Boolean(updateCommand),
    updateAgent:
      updateCommand?.stage === "pending" &&
      updateCommand.command.scope === "settings" &&
      updateCommand.command.type === "update-managed-agent"
        ? updateCommand.command.agent
        : undefined,
    permission,
    pending: command.stage === "pending",
    pendingDestinations: [...pendingDestinations],
    feedbackByDestination,
    promptSynchronization:
      promptCommand.stage === "succeeded" &&
      promptCommand.command.scope === "settings" &&
      promptCommand.command.type === "reset-agent-prompt-template"
        ? "accept-parent-value"
        : "preserve-local-draft",
    lensStageLabel: STAGE_LABEL[lens.stage],
    feedback: settingsFeedback(command, connection),
  };
}

export function targetSelectionViewModel(
  platform: DesktopPlatform,
  snapshot: AppSnapshot,
  command: CommandState,
  connectionMessage: string,
): TargetSelectionViewModel {
  return {
    platform,
    lens: snapshot.lens,
    pending: command.stage === "pending",
    message: presentationMessage(command, connectionMessage),
  };
}

export function overlayViewModel(
  platform: DesktopPlatform,
  snapshot: AppSnapshot,
  command: CommandState,
  connectionMessage: string,
): OverlayViewModel {
  return {
    platform,
    lens: snapshot.lens,
    pending: command.stage === "pending",
    sourceMetadata: snapshot.source_metadata,
    cancelPending: isPendingCommand(command, "overlay", "cancel"),
    lifecyclePending:
      command.stage === "pending" &&
      command.command.scope === "overlay" &&
      (command.command.type === "pause" ||
        command.command.type === "resume" ||
        command.command.type === "close")
        ? command.command.type
        : undefined,
    message: presentationMessage(command, connectionMessage),
  };
}
