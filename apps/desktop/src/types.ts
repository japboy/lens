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

export interface AgentPromptTemplate {
  schema_version: number;
  common: string;
  full_projection: string;
  source_checkpoint: string;
  current_projection_retry: string;
}

export interface AppConfig {
  agent_preferences?: { claude: AgentDefaults; codex: AgentDefaults };
  agent: AgentKind;
  working_directory: string;
  agent_prompt_template: AgentPromptTemplate;
}

export interface Bounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ResourceReference {
  uri: string;
  source_attribute: string;
}

export interface WindowIdentity {
  window_id: number;
  bundle_id: string;
  pid: number;
}

export interface WindowObservableFacts {
  title: string;
  application_name: string;
  frame: Bounds;
}

export type SelectedWindow = WindowIdentity & WindowObservableFacts;

export interface LensTarget {
  id: string;
  identity: WindowIdentity;
  facts_revision: number;
  facts: WindowObservableFacts;
}

export interface LensTargetSet {
  schema_version: number;
  selection_id: string;
  targets: LensTarget[];
}

export type LensTargetSelectionStage = "picking" | "reviewing";

export interface LensTargetSelectionItem {
  id: string;
  window: SelectedWindow;
  preview_uri?: string;
  preview_error?: string;
}

export interface LensTargetSelection {
  selection_id: string;
  stage: LensTargetSelectionStage;
  maximum_targets: number;
  anchor?: Bounds;
  items: LensTargetSelectionItem[];
  notice?: string;
}

export interface ExtractedNode {
  id: string;
  parent_id?: string;
  order: number;
  depth: number;
  role?: string;
  subrole?: string;
  title?: string;
  value?: string;
  description?: string;
  bounds?: Bounds;
  resource_refs?: ResourceReference[];
  children: string[];
}

export interface ExtractionResult {
  quality: ExtractionQuality;
  resolved_window?: {
    facts: WindowObservableFacts;
    resolution_score: number;
  };
  nodes: ExtractedNode[];
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
    resource_ref_count: number;
    resource_uri_bytes: number;
    omitted_resource_refs: number;
    resource_read_errors: number;
  };
}

export interface LensSource {
  application: string;
  window_title: string;
  bundle_id: string;
  window_id: number;
}

export type LensNodeKind =
  | "heading"
  | "paragraph"
  | "list"
  | "list_item"
  | "table"
  | "row"
  | "cell"
  | "link"
  | "control"
  | "dialog"
  | "region"
  | "text"
  | "image"
  | "unknown";

export type LensCoordinateSpace = "screen_points";

export interface LensNode {
  id: string;
  parent_id?: string;
  order: number;
  depth: number;
  kind: LensNodeKind;
  role?: string;
  subrole?: string;
  title?: string;
  value?: string;
  description?: string;
  bounds?: Bounds;
  coordinate_space?: LensCoordinateSpace;
  children?: string[];
  media_refs?: string[];
  resource_refs?: ResourceReference[];
}

export interface LensDocument {
  schema_version: number;
  source: LensSource;
  roots: string[];
  nodes: Record<string, LensNode>;
  quality: ExtractionQuality;
  omitted_resource_ref_count?: number;
  diagnostics: string[];
}

export type LensMediaScope = "ax_element_region" | "window_fallback";
export type LensMediaCoverage = "full_region" | "visible_subregion";

export interface LensMediaAttachment {
  id: string;
  target_id: string;
  uri: string;
  scope: LensMediaScope;
  source_node_id?: string;
  source_bounds: Bounds;
  captured_bounds: Bounds;
  coverage: LensMediaCoverage;
  coordinate_space: LensCoordinateSpace;
  mime_type: string;
  pixel_width: number;
  pixel_height: number;
  encoded_bytes: number;
}

export interface LensMediaOmission {
  target_id: string;
  attachment_id?: string;
  source_node_id?: string;
  reason:
    | "missing_bounds"
    | "invalid_bounds"
    | "outside_window"
    | "attachment_limit"
    | "byte_budget"
    | "capture_failed";
  omitted_count: number;
  first_order?: number;
  last_order?: number;
  detail: string;
}

export interface LensAccessibilitySource {
  source_id: string;
  target_id: string;
  revision: number;
  source: LensSource;
  capture: ExtractionResult;
  document?: LensDocument;
  quality: ExtractionQuality;
}

export interface LensContext {
  schema_version: number;
  context_id: string;
  revision: number;
  sources: LensAccessibilitySource[];
  media: LensMediaAttachment[];
  media_omissions: LensMediaOmission[];
  quality: ExtractionQuality;
  diagnostics: string[];
}

export interface LensContentNode {
  id: string;
  parent_id?: string;
  kind: LensNodeKind;
  role?: string;
  subrole?: string;
  title?: string;
  value?: string;
  description?: string;
  media_refs?: string[];
  resource_refs?: ResourceReference[];
}

export interface LensDocumentProjection {
  nodes: LensContentNode[];
}

export interface ProjectionOmission {
  reason: "application_chrome" | "token_budget" | "unsupported_semantics" | "resource_budget";
  omitted_node_count: number;
  first_order?: number;
  last_order?: number;
  detail?: string;
}

export interface LensInputSource {
  source_id: string;
  target_id: string;
  source_revision: number;
  source: LensSource;
  document?: LensDocumentProjection;
  quality: ExtractionQuality;
  omissions: ProjectionOmission[];
}

export interface LensInput {
  schema_version: number;
  context_id: string;
  context_revision: number;
  sources: LensInputSource[];
  media: LensMediaAttachment[];
  media_omissions: LensMediaOmission[];
  quality: ExtractionQuality;
}

export interface AgentAuthMethod {
  id: string;
  name: string;
  description?: string;
  kind: "agent" | "terminal" | "environment_variable";
  supported: boolean;
}

export interface AgentSelectionState {
  config_options?: SessionConfigOption[];
  modes?: SessionMode[];
  policy_default?: string;
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
  progress_text?: string;
  stop_reason?: string;
  authentication_message?: string;
}

interface LensOutputBlockBase {
  message_id?: string;
}

export interface LensMarkdownOutputBlock extends LensOutputBlockBase {
  type: "markdown";
  text: string;
}

export interface LensImageOutputBlock extends LensOutputBlockBase {
  type: "image";
  mime_type: string;
  data: string;
  uri?: string;
}

export interface LensUnsupportedOutputBlock extends LensOutputBlockBase {
  type: "unsupported";
  content_type: string;
}

export type LensOutputBlock =
  | LensMarkdownOutputBlock
  | LensImageOutputBlock
  | LensUnsupportedOutputBlock;

export interface ProjectionRef {
  revision: number;
  digest: string;
}

export interface LensRepresentation {
  representation_id: string;
  context_id: string;
  context_revision: number;
  projection: ProjectionRef;
  run_id: string;
  output_blocks: LensOutputBlock[];
}

export type LensLiveLifecycle = "watching" | "paused" | "stopped";
export type LensLiveHealth = "healthy" | "degraded" | "unavailable";
export type LensLiveFreshness = "none" | "checking" | "current" | "stale" | "unverified";
export type LensLiveOutcome = "unchanged" | "updated" | "failed";

export interface LensLiveState {
  lifecycle: LensLiveLifecycle;
  health: LensLiveHealth;
  freshness: LensLiveFreshness;
  agent_refresh_interval_seconds: number;
  last_outcome?: LensLiveOutcome;
  error?: string;
}

export interface LensState {
  session_controls?: AgentSessionControlState;
  operation_id?: string;
  stage: LensStage;
  selection?: LensTargetSelection;
  target_set?: LensTargetSet;
  context?: LensContext;
  input?: LensInput;
  projection?: ProjectionRef;
  representation?: LensRepresentation;
  live?: LensLiveState;
  output_blocks: LensOutputBlock[];
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

export interface SessionChoice {
  value: string;
  name: string;
  description?: string;
}
export interface SessionChoiceGroup {
  group: string;
  name: string;
  options: SessionChoice[];
}
export interface SessionConfigOption {
  id: string;
  name: string;
  description?: string;
  category?: string;
  type: "select" | "boolean";
  currentValue: string | boolean;
  options?: (SessionChoice | SessionChoiceGroup)[];
}
export interface SessionMode {
  id: string;
  name: string;
  description?: string;
}
export type ToolPolicy = "ask" | "allow" | "deny";
export interface ToolPolicies {
  read: ToolPolicy;
  search: ToolPolicy;
  fetch: ToolPolicy;
  edit: ToolPolicy;
  delete: ToolPolicy;
  move: ToolPolicy;
  execute: ToolPolicy;
}
export interface AgentDefaults {
  choices: { config_id: string; value: string }[];
  tools: ToolPolicies;
}
export type InteractionResponse =
  | { action: "accept" | "decline" | "cancel" }
  | { action: "select"; option_id: string }
  | { action: "submit"; content: Record<string, string | number | boolean | string[]> };
export interface AgentInteraction {
  id: string;
  run_id?: string;
  sequence: number;
  status: "pending" | "accepted" | "declined" | "cancelled" | "expired";
  details?:
    | { kind: "form"; message: string; schema: ElicitationSchema }
    | { kind: "url"; message: string; elicitation_id: string; url: string }
    | { kind: "mode_transition"; from: string; to: string }
    | {
        kind: "permission";
        tool_call_id: string;
        title: string;
        effect: string;
        arguments: unknown;
        options: { optionId: string; name: string; kind: "allow_once" | "reject_once" }[];
      };
}
export interface AgentSessionControlState {
  instance_id: string;
  operation_id: string;
  session_id: string;
  agent_name: string;
  active: boolean;
  config_revision: number;
  notice?: string;
  config_options?: SessionConfigOption[];
  modes: SessionMode[];
  effective_mode: string;
  configured_mode?: string;
  configured_origin?: "policy" | "user" | "agent";
  last_mode_origin?: "policy" | "user" | "agent";
  policy_default: string;
  change?: {
    config_id: string;
    value: string;
    status: "pending" | "succeeded" | "rejected" | "failed";
  };
  interactions: AgentInteraction[];
}

export interface ElicitationField {
  type: "string" | "number" | "integer" | "boolean" | "array";
  title?: string;
  description?: string;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  enum?: string[];
  oneOf?: { const: string; title: string }[];
  items?: { enum?: string[]; oneOf?: { const: string; title: string }[] };
  default?: string | number | boolean | string[];
}
export interface ElicitationSchema {
  type: "object";
  properties: Record<string, ElicitationField>;
  required?: string[];
  title?: string;
  description?: string;
}
