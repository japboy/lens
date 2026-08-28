import type {
  AgentAuthMethod,
  AgentKind,
  AgentRuntimeStage,
  AgentSelectionStage,
  AgentSelectionState,
  LensImageOutputBlock,
  LensMediaAttachment,
  LensOutputBlock,
  LensStage,
  LensState,
} from "./types";

export const AGENT_RUNTIME_LABEL: Record<AgentRuntimeStage, string> = {
  not_installed: "Agent runtime is not installed",
  resolving: "Resolving approved Agent runtime…",
  downloading: "Downloading approved runtime…",
  verifying: "Verifying runtime integrity…",
  installing: "Installing Agent runtime…",
  ready: "Agent runtime installed and verified",
  failed: "Agent runtime installation failed",
};

export function isAgentRuntimeActive(stage: AgentRuntimeStage): boolean {
  switch (stage) {
    case "resolving":
    case "downloading":
    case "verifying":
    case "installing":
      return true;
    case "not_installed":
    case "ready":
    case "failed":
      return false;
  }
}

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
  selecting: "Selecting windows…",
  extracting: "Extracting accessibility content…",
  ready: "Extraction complete",
  connecting: "Connecting to the agent…",
  authentication_required: "Agent authentication required",
  transforming: "Transforming content…",
  completed: "Transformation complete",
  cancelled: "Cancelled",
  failed: "Unable to complete the operation",
};

const SUPPORTED_IMAGE_MIME_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "image/avif",
]);

export function lensOutputBlocks(lens: LensState): LensOutputBlock[] {
  return lens.output_blocks;
}

export function imageDataUrl(block: LensImageOutputBlock): string | undefined {
  const mimeType = block.mime_type.toLowerCase();
  return SUPPORTED_IMAGE_MIME_TYPES.has(mimeType)
    ? `data:${mimeType};base64,${block.data}`
    : undefined;
}

export function lensSourceJson(lens: LensState): string {
  return lens.input ? JSON.stringify(lens.input, undefined, 2) : "";
}

export function inputMediaPreviewUrl(
  lens: LensState,
  attachment: LensMediaAttachment,
): string | undefined {
  const input = lens.input;
  if (
    !input ||
    lens.operation_id !== input.context_id ||
    attachment.mime_type.toLowerCase() !== "image/png"
  ) {
    return undefined;
  }
  const expected = `lens://context/${input.context_id}/${input.context_revision}/media/${attachment.id}`;
  return attachment.uri === expected ? expected : undefined;
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
