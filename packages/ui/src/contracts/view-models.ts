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

export interface HistoryEntry {
  id: string;
  title: string;
  agent: string;
  cwd: string;
  updated_at: string | null;
  state: "listed" | "local_only" | "not_seen";
  /** Matching current source identity, not runtime reachability or load capability. */
  available: boolean;
}

export interface HistoryPage {
  revision: number;
  total: number;
  entries: HistoryEntry[];
}

/** Sync counts Agent/directory targets; deletion counts sessions. Purged counts cache rows. */
export interface HistoryResult {
  succeeded: number;
  failed: number;
  purged: number;
  errors: string[];
}

export interface HistoryPresentation {
  stage: "inactive" | "loading" | "ready" | "failed";
  offset: number;
  page?: HistoryPage;
  error?: string;
  readError?: string;
  operation: "idle" | "confirming" | "opening" | "deleting" | "synchronizing";
  outcome?: { kind: "sync" | "delete"; result: HistoryResult };
}

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
