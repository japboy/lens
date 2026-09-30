import type {
  AppBridge,
  McpUiHostCapabilities,
  McpUiResourceMeta,
} from "@modelcontextprotocol/ext-apps/app-bridge";

/** Immutable identity; resource bodies are fetched only when a selected App opens. */
export interface McpAppDescriptor {
  id: string;
  operation_id: string;
  session_id: string;
  server_id: string;
  tool_name: string;
  resource_uri: string;
  title?: string;
}

export interface McpAppsServer {
  id: string;
  name: string;
  url: string;
}

export interface McpAppLease {
  id: string;
  artifact_id: string;
  generation: string;
  proxy_url: string;
  proxy_origin: string;
  resource: { html: string; meta?: { ui?: McpUiResourceMeta } };
  input: Record<string, unknown>;
  result: Parameters<AppBridge["sendToolResult"]>[0];
  host_capabilities: McpUiHostCapabilities;
  live: boolean;
  document_url?: string;
}

export interface McpAppsPort {
  openMcpApp(artifactId: string, hostOrigin: string): Promise<McpAppLease>;
  mcpAppRequest(
    leaseId: string,
    request: { method: string; params?: unknown },
  ): Promise<{ result: unknown; draft?: McpAppMessageDraft }>;
  closeMcpApp(leaseId: string): Promise<void>;
  submitMcpAppMessage(leaseId: string, draftId: string): Promise<void>;
}

export interface McpAppMessageDraft {
  id: string;
  text: string;
}

export type McpAppState =
  | { stage: "idle" }
  | { stage: "opening"; artifactId: string }
  | { stage: "initializing"; artifactId: string }
  | { stage: "ready"; artifactId: string; live: boolean }
  | { stage: "closing"; artifactId: string }
  | { stage: "closed"; artifactId: string }
  | { stage: "failed"; artifactId: string; message: string };
