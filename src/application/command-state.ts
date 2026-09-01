import type { OverlayIntent, SettingsIntent, TargetSelectionIntent } from "../components/events";

export type CommandIdentity =
  | { scope: "settings"; type: SettingsIntent["type"] }
  | { scope: "target-selection"; type: TargetSelectionIntent["type"] }
  | { scope: "overlay"; type: OverlayIntent["type"] };

export type CommandState =
  | { stage: "idle" }
  | { stage: "pending"; command: CommandIdentity }
  | { stage: "succeeded"; command: CommandIdentity; message: string }
  | { stage: "failed"; command: CommandIdentity; message: string };

export const IDLE_COMMAND_STATE: CommandState = { stage: "idle" };

export function canStartCommand(state: CommandState, next: CommandIdentity): boolean {
  if (state.stage !== "pending") return true;
  if (next.scope !== "overlay") return false;
  if (state.command.scope === next.scope && state.command.type === next.type) {
    return false;
  }
  if (
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
