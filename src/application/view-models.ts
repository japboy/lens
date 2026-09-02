import type { DesktopPlatform } from "../presentation-context";
import type { SettingsDestination } from "../agent-prompt-template";
import type {
  AgentRuntimeState,
  AgentSelectionState,
  AppConfig,
  AppSnapshot,
  LensState,
} from "../types";
import { STAGE_LABEL } from "../view-model";
import type { AccessibilityPermissionState } from "./accessibility-permission-controller";
import { snapshotConnectionMessage, type SnapshotConnectionState } from "./app-snapshot-controller";
import {
  commandMessage,
  isPendingCommand,
  type CommandIdentity,
  type CommandState,
} from "./command-state";

export type PromptSynchronization = "preserve-local-draft" | "accept-parent-value";

export type SettingsFeedbackTarget = "application" | SettingsDestination;

export type SettingsFeedback =
  | { stage: "none" }
  | {
      stage: "status" | "error";
      target: SettingsFeedbackTarget;
      message: string;
    };

export type SettingsFeedbackMessage = Exclude<SettingsFeedback, { stage: "none" }>;

const DEFAULT_AGENT_SELECTION: AgentSelectionState = {
  stage: "unselected",
  auth_methods: [],
};
const DEFAULT_AGENT_RUNTIME: AgentRuntimeState = {
  stage: "not_installed",
  downloaded_bytes: 0,
};
const DEFAULT_LENS: LensState = { stage: "idle", output_blocks: [] };

export interface SettingsViewModel {
  platform: DesktopPlatform;
  config?: AppConfig;
  agentSelection: AgentSelectionState;
  agentRuntime: AgentRuntimeState;
  permission: AccessibilityPermissionState;
  pending: boolean;
  promptSynchronization: PromptSynchronization;
  lensStageLabel: string;
  feedback: SettingsFeedback;
}

export interface TargetSelectionViewModel {
  platform: DesktopPlatform;
  lens: LensState;
  pending: boolean;
  message: string;
}

export interface OverlayViewModel {
  platform: DesktopPlatform;
  lens: LensState;
  pending: boolean;
  cancelPending: boolean;
  message: string;
}

function presentationMessage(command: CommandState, connectionMessage: string): string {
  return commandMessage(command) || connectionMessage;
}

type SettingsCommandType = Extract<CommandIdentity, { scope: "settings" }>["type"];

function settingsFeedbackTarget(command: SettingsCommandType): SettingsDestination {
  switch (command) {
    case "select-agent":
    case "authenticate-agent-selection":
    case "reauthenticate-agent-selection":
    case "sign-out-agent-selection":
    case "choose-directory":
    case "request-accessibility-permission":
      return "general";
    case "save-agent-prompt-template":
    case "reset-agent-prompt-template":
      return "agent-prompt";
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

function settingsFeedback(
  command: CommandState,
  connection: SnapshotConnectionState,
): SettingsFeedback {
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
  return connectionFeedback(connection);
}

export function settingsViewModel(
  platform: DesktopPlatform,
  snapshot: AppSnapshot | undefined,
  permission: AccessibilityPermissionState,
  command: CommandState,
  connection: SnapshotConnectionState,
): SettingsViewModel {
  const lens = snapshot?.lens ?? DEFAULT_LENS;
  return {
    platform,
    config: snapshot?.config,
    agentSelection: snapshot?.agent_selection ?? DEFAULT_AGENT_SELECTION,
    agentRuntime: snapshot?.agent_runtime ?? DEFAULT_AGENT_RUNTIME,
    permission,
    pending: command.stage === "pending",
    promptSynchronization:
      command.stage === "succeeded" &&
      command.command.scope === "settings" &&
      command.command.type === "reset-agent-prompt-template"
        ? "accept-parent-value"
        : "preserve-local-draft",
    lensStageLabel: STAGE_LABEL[lens.stage],
    feedback: settingsFeedback(command, connection),
  };
}

export function targetSelectionViewModel(
  platform: DesktopPlatform,
  snapshot: AppSnapshot | undefined,
  command: CommandState,
  connectionMessage: string,
): TargetSelectionViewModel {
  return {
    platform,
    lens: snapshot?.lens ?? DEFAULT_LENS,
    pending: command.stage === "pending",
    message: presentationMessage(command, connectionMessage),
  };
}

export function overlayViewModel(
  platform: DesktopPlatform,
  snapshot: AppSnapshot | undefined,
  command: CommandState,
  connectionMessage: string,
): OverlayViewModel {
  return {
    platform,
    lens: snapshot?.lens ?? DEFAULT_LENS,
    pending: command.stage === "pending",
    cancelPending: isPendingCommand(command, "overlay", "cancel"),
    message: presentationMessage(command, connectionMessage),
  };
}
