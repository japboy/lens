import type { DesktopPlatform } from "./context";
import type { ResourceState, AccessibilityPermissionState } from "./resource-state";
import type { SettingsDestination } from "../presentation/agent-prompt-template";
import type {
  AgentRuntimeState,
  ManagedAgentKind,
  AgentSelectionState,
  AppConfig,
  LensState,
  SourceMetadata,
} from "./lens";
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

export interface SettingsViewModel {
  platform: DesktopPlatform;
  config?: AppConfig;
  agentSelection: AgentSelectionState;
  agentRuntime: AgentRuntimeState;
  updatePending: boolean;
  updateAgent?: ManagedAgentKind;
  permission: AccessibilityPermissionState;
  pending: boolean;
  pendingDestinations?: SettingsDestination[];
  feedbackByDestination?: Partial<Record<SettingsDestination, SettingsFeedback>>;
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

export interface InteractionSubmission {
  instanceId: string;
  interactionId: string;
  stage: "sending" | "sent" | "failed";
  message?: string;
}

export interface OverlayViewModel {
  sourceResource?: ResourceState;
  sourceMetadata?: SourceMetadata | null;
  outputResource?: ResourceState;
  interactionSubmission?: InteractionSubmission;
  platform: DesktopPlatform;
  lens: LensState;
  pending: boolean;
  cancelPending: boolean;
  lifecyclePending?: "pause" | "resume" | "close";
  message: string;
}
