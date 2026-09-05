import type { AgentKind, AgentPromptTemplate, AgentDefaults, InteractionResponse } from "../types";

export const AGENT_INTENT_EVENT = "lens-agent-intent";
export const PROMPT_INTENT_EVENT = "lens-prompt-intent";
export const SETTINGS_INTENT_EVENT = "lens-settings-intent";
export const TARGET_REMOVE_EVENT = "lens-target-remove";
export const TARGET_SELECTION_INTENT_EVENT = "lens-target-selection-intent";
export const AGENT_OUTPUT_INTENT_EVENT = "lens-agent-output-intent";
export const OVERLAY_INTENT_EVENT = "lens-overlay-intent";

export type AgentIntent =
  | { type: "preview-model"; configId: string; value?: string }
  | { type: "save-defaults"; defaults: AgentDefaults }
  | { type: "select"; agent: AgentKind }
  | { type: "authenticate"; methodId: string }
  | { type: "reauthenticate" }
  | { type: "sign-out" };

export type PromptIntent =
  | { type: "save"; agentPromptTemplate: AgentPromptTemplate }
  | { type: "reset" };

export type SettingsIntent =
  | { type: "preview-agent-model"; configId: string; value?: string }
  | { type: "save-agent-defaults"; defaults: AgentDefaults }
  | { type: "select-agent"; agent: AgentKind }
  | { type: "authenticate-agent-selection"; methodId: string }
  | { type: "reauthenticate-agent-selection" }
  | { type: "sign-out-agent-selection" }
  | { type: "save-agent-prompt-template"; agentPromptTemplate: AgentPromptTemplate }
  | { type: "reset-agent-prompt-template" }
  | { type: "choose-directory" }
  | { type: "request-accessibility-permission" };

export type TargetSelectionIntent =
  | { type: "add" }
  | { type: "remove"; targetId: string }
  | { type: "confirm" };

export type AgentOutputIntent =
  | { type: "open-external-url"; url: string }
  | { type: "report-error"; message: string };

export type OverlayIntent =
  | {
      type: "set-session-option";
      instanceId: string;
      revision: number;
      configId: string;
      value: string;
    }
  | {
      type: "respond-interaction";
      instanceId: string;
      interactionId: string;
      response: InteractionResponse;
    }
  | { type: "authenticate"; methodId: string }
  | { type: "retry" }
  | { type: "cancel" }
  | { type: "pause" }
  | { type: "resume" }
  | { type: "close" }
  | { type: "open-external-url"; url: string }
  | { type: "report-error"; message: string };

export function dispatchComponentEvent<T>(target: EventTarget, type: string, detail: T): boolean {
  return target.dispatchEvent(
    new CustomEvent<T>(type, {
      bubbles: true,
      composed: true,
      detail,
    }),
  );
}
