import type {
  AppConfig,
  AgentSelectionState,
  AgentRuntimeState,
  LensState,
  SourceMetadata,
} from "ui/contracts/lens";

export interface AppSnapshot {
  source_ref: string | null;
  source_metadata: SourceMetadata | null;
  output_ref: string | null;
  revision: number;
  config: AppConfig;
  agent_selection: AgentSelectionState;
  agent_runtime: AgentRuntimeState;
  lens: LensState;
}
