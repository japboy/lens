import type { OverlayIntent, SettingsIntent, TargetSelectionIntent } from "../components/events";
import type { ManagedAgentKind } from "../types";

export type CommandIdentity =
  | {
      scope: "settings";
      type: Exclude<SettingsIntent["type"], "open-about" | "update-managed-agent">;
    }
  | { scope: "settings"; type: "update-managed-agent"; agent: ManagedAgentKind }
  | { scope: "target-selection"; type: TargetSelectionIntent["type"] }
  | { scope: "overlay"; type: OverlayIntent["type"] };

export type CommandState =
  | { stage: "idle" }
  | { stage: "pending"; command: CommandIdentity }
  | { stage: "succeeded"; command: CommandIdentity; message: string }
  | { stage: "failed"; command: CommandIdentity; message: string };

export const IDLE_COMMAND_STATE: CommandState = { stage: "idle" };

/** Finite conflict groups: connection changes invalidate session-default authority. */
export type CommandLane =
  | "settings:prompt"
  | "settings:privacy"
  | "settings:connection"
  | "overlay:lifecycle"
  | "overlay:link"
  | "overlay:option"
  | "overlay:interaction"
  | "overlay:agent"
  | "target-selection";

export function commandLane(command: CommandIdentity): CommandLane {
  if (command.scope === "settings") {
    switch (command.type) {
      case "update-prompt-presets":
      case "save-agent-prompt-template":
      case "reset-agent-prompt-template":
        return "settings:prompt";
      case "request-accessibility-permission":
      case "open-screen-recording-settings":
        return "settings:privacy";
      case "preview-agent-model":
      case "save-agent-defaults":
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
        return "settings:connection";
    }
  }
  if (command.scope === "overlay") {
    switch (command.type) {
      case "pause":
      case "resume":
      case "close":
        return "overlay:lifecycle";
      case "open-external-url":
        return "overlay:link";
      case "set-session-option":
        return "overlay:option";
      case "respond-interaction":
        return "overlay:interaction";
      case "authenticate":
      case "retry":
      case "cancel":
      case "report-error":
        return "overlay:agent";
    }
  }
  if (command.scope === "target-selection") return "target-selection";
  throw new Error(`Unhandled command: ${JSON.stringify(command satisfies never)}`);
}

export function canStartCommand(state: CommandState, next: CommandIdentity): boolean {
  if (state.stage !== "pending") return true;
  if (next.scope === "settings" && state.command.scope === "settings")
    return commandLane(next) !== commandLane(state.command);
  if (next.scope !== "overlay") return false;
  if (state.command.scope === "overlay" && state.command.type === "close")
    return next.type === "open-external-url";
  if (
    state.command.scope === "overlay" &&
    (state.command.type === "pause" || state.command.type === "resume") &&
    (next.type === "pause" || next.type === "resume")
  )
    return false;
  if (state.command.scope === next.scope && state.command.type === next.type) {
    return false;
  }
  if (
    next.type === "respond-interaction" ||
    next.type === "set-session-option" ||
    next.type === "close" ||
    next.type === "pause" ||
    next.type === "resume" ||
    next.type === "open-external-url"
  ) {
    return true;
  }
  if (next.type !== "cancel" || state.command.scope !== "overlay") return false;
  return state.command.type === "authenticate" || state.command.type === "retry";
}

export function isPendingCommand(
  state: CommandState,
  scope: CommandIdentity["scope"],
  type: CommandIdentity["type"],
): boolean {
  return state.stage === "pending" && state.command.scope === scope && state.command.type === type;
}

export function commandMessage(state: CommandState): string {
  return state.stage === "succeeded" || state.stage === "failed" ? state.message : "";
}
