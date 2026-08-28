import type { DesktopPlatform } from "../presentation-context";
import type {
  AgentRuntimeState,
  AgentSelectionState,
  AppConfig,
  AppSnapshot,
  LensState,
} from "../types";
import { STAGE_LABEL } from "../view-model";
import type { AccessibilityPermissionState } from "./accessibility-permission-controller";
import { commandMessage, type CommandState } from "./command-state";

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
  message: string;
  lensStageLabel: string;
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
  message: string;
}

function presentationMessage(command: CommandState, connectionMessage: string): string {
  return commandMessage(command) || connectionMessage;
}

export function settingsViewModel(
  platform: DesktopPlatform,
  snapshot: AppSnapshot | undefined,
  permission: AccessibilityPermissionState,
  command: CommandState,
  connectionMessage: string,
): SettingsViewModel {
  const lens = snapshot?.lens ?? DEFAULT_LENS;
  return {
    platform,
    config: snapshot?.config,
    agentSelection: snapshot?.agent_selection ?? DEFAULT_AGENT_SELECTION,
    agentRuntime: snapshot?.agent_runtime ?? DEFAULT_AGENT_RUNTIME,
    permission,
    pending: command.stage === "pending",
    message: presentationMessage(command, connectionMessage),
    lensStageLabel: STAGE_LABEL[lens.stage],
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
    message: presentationMessage(command, connectionMessage),
  };
}
