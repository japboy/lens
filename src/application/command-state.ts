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

export function commandMessage(state: CommandState): string {
  return state.stage === "succeeded" || state.stage === "failed" ? state.message : "";
}
