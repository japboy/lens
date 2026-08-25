import type {
  AgentAuthMethod,
  AgentKind,
  AgentSelectionStage,
  AgentSelectionState,
  LensStage,
  LensState,
} from "./types";

export const AGENT_SELECTION_LABEL: Record<AgentSelectionStage, string> = {
  unselected: "Select an Agent to verify authentication.",
  checking: "Checking Agent authentication…",
  authentication_required: "Agent authentication required",
  authenticating: "Authenticating Agent…",
  signing_out: "Signing out of Agent…",
  selected: "Agent authenticated and selected",
  failed: "Agent selection failed",
};

export function selectedAgent(selection: AgentSelectionState): AgentKind | undefined {
  return selection.stage === "selected" ? selection.candidate : undefined;
}

export const STAGE_LABEL: Record<LensStage, string> = {
  idle: "Idle",
  selecting: "Selecting a window…",
  extracting: "Extracting accessibility content…",
  ready: "Extraction complete",
  connecting: "Connecting to the agent…",
  authentication_required: "Agent authentication required",
  transforming: "Transforming content…",
  completed: "Transformation complete",
  cancelled: "Cancelled",
  failed: "Unable to complete the operation",
};

export function lensTranslationText(lens: LensState): string {
  return lens.transformed_text ?? "";
}

export function supportedAuthMethods(lens: LensState): AgentAuthMethod[] {
  return lens.agent?.auth_methods.filter((method) => method.supported) ?? [];
}

export function shouldApplySnapshot(currentRevision: number, nextRevision: number): boolean {
  return Number.isSafeInteger(nextRevision) && nextRevision >= 0 && nextRevision > currentRevision;
}

export function showsLensProgress(stage: LensStage): boolean {
  switch (stage) {
    case "selecting":
    case "extracting":
    case "ready":
    case "connecting":
    case "transforming":
      return true;
    case "idle":
    case "authentication_required":
    case "completed":
    case "cancelled":
    case "failed":
      return false;
  }
}
