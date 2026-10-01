import type {
  AppBridge,
  McpUiHostCapabilities,
  McpUiResourceMeta,
} from "@modelcontextprotocol/ext-apps/app-bridge";

/** Product metadata and optional presentation preparation are composition-owned. */
export interface McpAppHostOptions {
  hostInfo: ConstructorParameters<typeof AppBridge>[1];
  frameTitle: string;
  getHostContext: (
    lease: McpAppLease,
    owner: Window,
  ) => NonNullable<ConstructorParameters<typeof AppBridge>[3]>["hostContext"];
  prepareDocument?: (lease: McpAppLease, descriptor: McpAppDescriptor) => Promise<void> | undefined;
}

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
  ): Promise<{ result: unknown; draft?: McpAppMessageDraft; link?: McpAppLinkDraft }>;
  closeMcpApp(leaseId: string): Promise<void>;
  submitMcpAppMessage(leaseId: string, draftId: string): Promise<void>;
  submitMcpAppLink(leaseId: string, linkId: string): Promise<void>;
}

export interface McpAppLinkDraft {
  id: string;
  url: string;
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
