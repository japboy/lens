import type {
  AgentAuthMethod,
  AgentKind,
  AgentRuntimeStage,
  AgentSelectionStage,
  AgentSelectionState,
  LensImageOutputBlock,
  LensLiveState,
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

export interface LensProgressSnackbar {
  readonly title: string;
  readonly detail: string;
}

export function lensProgressSnackbar(stage: LensStage): LensProgressSnackbar | undefined {
  switch (stage) {
    case "selecting":
      return {
        title: STAGE_LABEL[stage],
        detail: "Choose the windows Lens should use.",
      };
    case "extracting":
      return {
        title: STAGE_LABEL[stage],
        detail: "Reading content from the selected windows.",
      };
    case "connecting":
      return {
        title: STAGE_LABEL[stage],
        detail: "Preparing the Agent session.",
      };
    case "transforming":
      return {
        title: STAGE_LABEL[stage],
        detail: "Interpretation updates appear as Agent output arrives.",
      };
    case "ready":
      return {
        title: "Preparing Agent…",
        detail: "The initial transformation starts automatically.",
      };
    case "idle":
    case "authentication_required":
    case "completed":
    case "cancelled":
    case "failed":
      return undefined;
  }
}

const SUPPORTED_IMAGE_MIME_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "image/avif",
]);

export function lensOutputBlocks(lens: LensState): LensOutputBlock[] {
  return lens.representation?.output_blocks ?? lens.output_blocks;
}

export type LensOutputMode = "empty" | "initial-stream" | "settled";

export interface LensOutputPresentation {
  readonly published?: { operationId: string; representationId: string };
  readonly blocks: LensOutputBlock[];
  readonly identity?: string;
  readonly mode: LensOutputMode;
}

export function lensOutputPresentation(lens: LensState): LensOutputPresentation {
  const representation = lens.representation;
  if (representation) {
    return {
      blocks: representation.output_blocks,
      ...(lens.operation_id
        ? {
            published: {
              operationId: lens.operation_id,
              representationId: representation.representation_id,
            },
          }
        : {}),
      identity: representation.representation_id,
      mode: representation.output_blocks.length ? "settled" : "empty",
    };
  }
  if (!lens.output_blocks.length) {
    return { blocks: [], identity: lens.operation_id, mode: "empty" };
  }
  return {
    blocks: lens.output_blocks,
    identity: lens.operation_id,
    mode: lens.stage === "transforming" ? "initial-stream" : "settled",
  };
}

export interface LensLiveStatus {
  readonly title:
    | "Watching"
    | "Update queued"
    | "Updating"
    | "Updated"
    | "Paused"
    | "Stopped"
    | "Needs Attention";
  readonly detail: string;
  readonly busy: boolean;
  readonly prominent: boolean;
}

function agentRefreshIntervalLabel(seconds: number): string {
  const minutes = seconds / 60;
  return Number.isInteger(minutes)
    ? `${minutes} ${minutes === 1 ? "minute" : "minutes"}`
    : `${seconds} ${seconds === 1 ? "second" : "seconds"}`;
}

export function lensLiveStatus(live: LensLiveState | undefined): LensLiveStatus | undefined {
  if (!live) return undefined;
  switch (live.lifecycle) {
    case "paused":
      return {
        title: "Paused",
        detail:
          live.freshness === "unverified"
            ? "Automatic updates are paused. The displayed interpretation is retained and unverified."
            : "Automatic updates are paused. The displayed interpretation remains available.",
        busy: false,
        prominent: false,
      };
    case "stopped":
      return {
        title: "Stopped",
        detail: "Automatic updates have stopped.",
        busy: false,
        prominent: false,
      };
    case "watching":
      break;
  }

  if (
    live.health === "unavailable" ||
    live.freshness === "unverified" ||
    live.last_outcome === "failed"
  ) {
    return {
      title: "Needs Attention",
      detail:
        live.error ??
        (live.health === "unavailable"
          ? "Automatic monitoring is unavailable."
          : "The displayed interpretation could not be verified against the latest content."),
      busy: false,
      prominent: true,
    };
  }

  switch (live.freshness) {
    case "checking":
      return {
        title: "Updating",
        detail: "Checking the selected windows for meaningful changes.",
        busy: true,
        prominent: true,
      };
    case "stale":
      return {
        title: "Update queued",
        detail: `The latest source change will be applied automatically. Agent updates start at most once every ${agentRefreshIntervalLabel(live.agent_refresh_interval_seconds)}.`,
        busy: false,
        prominent: true,
      };
    case "none":
    case "current":
      if (live.last_outcome === "updated") {
        return {
          title: "Updated",
          detail: "The latest interpretation was applied automatically.",
          busy: false,
          prominent: false,
        };
      }
      return {
        title: "Watching",
        detail:
          live.health === "degraded"
            ? "Watching for content changes. Updates may be delayed because monitoring coverage is degraded."
            : "Watching for content changes that may be sent to the selected Agent.",
        busy: false,
        prominent: live.health === "degraded",
      };
  }
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
