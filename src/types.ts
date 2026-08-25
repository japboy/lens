export type AgentKind = "claude" | "codex";
export type AgentSelectionStage =
  | "unselected"
  | "checking"
  | "authentication_required"
  | "authenticating"
  | "signing_out"
  | "selected"
  | "failed";
export type AgentRuntimeStage =
  | "not_installed"
  | "resolving"
  | "downloading"
  | "verifying"
  | "installing"
  | "ready"
  | "failed";
export type ExtractionQuality = "full" | "partial" | "unavailable";
export type LensStage =
  | "idle"
  | "selecting"
  | "extracting"
  | "ready"
  | "connecting"
  | "authentication_required"
  | "transforming"
  | "completed"
  | "cancelled"
  | "failed";

export interface AppConfig {
  agent: AgentKind;
  working_directory: string;
}

export interface Bounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface SelectedWindow {
  window_id: number;
  title: string;
  application_name: string;
  bundle_id: string;
  pid: number;
  frame: Bounds;
}

export interface ExtractionResult {
  quality: ExtractionQuality;
  text: string;
  diagnostics: string[];
  metrics: {
    visited_nodes: number;
    text_bytes: number;
    offscreen_text_nodes: number;
    virtualization_signals: number;
    truncated_nodes: boolean;
    truncated_text: boolean;
    children_read_errors: number;
  };
}

export interface LensInput {
  source: {
    application: string;
    window_title: string;
    bundle_id: string;
    window_id: number;
  };
  text: string;
  extraction_quality: ExtractionQuality;
}

export interface AgentAuthMethod {
  id: string;
  name: string;
  description?: string;
  kind: "agent" | "terminal" | "environment_variable";
  supported: boolean;
}

export interface AgentSelectionState {
  operation_id?: string;
  stage: AgentSelectionStage;
  candidate?: AgentKind;
  auth_methods: AgentAuthMethod[];
  message?: string;
  error?: string;
}

export interface AgentRuntimeState {
  operation_id?: string;
  stage: AgentRuntimeStage;
  agent?: AgentKind;
  version?: string;
  downloaded_bytes: number;
  total_bytes?: number;
  message?: string;
  error?: string;
}

export interface AgentRunState {
  run_id: string;
  kind: AgentKind;
  adapter_name: string;
  adapter_version: string;
  session_id?: string;
  session_mode_id?: string;
  auth_methods: AgentAuthMethod[];
  received_updates: number;
  stop_reason?: string;
  authentication_message?: string;
}

export interface LensState {
  operation_id?: string;
  stage: LensStage;
  target?: SelectedWindow;
  extraction?: ExtractionResult;
  input?: LensInput;
  transformed_text?: string;
  agent?: AgentRunState;
  error?: string;
}

export interface AppSnapshot {
  revision: number;
  config: AppConfig;
  agent_selection: AgentSelectionState;
  agent_runtime: AgentRuntimeState;
  lens: LensState;
}
