import type { McpAppsPort, McpAppHostOptions, McpAppLease } from "adapter-mcp-apps-host";
export interface UiMcpAppsPort extends McpAppsPort {
  readonly presentation: McpAppHostOptions;
  openHtmlPresentation(source: HtmlPresentationSource, hostOrigin: string): Promise<McpAppLease>;
}
export type HtmlPresentationSource =
  | {
      kind: "live";
      output_ref: { operation_id: string; representation_id: string };
      block_index: number;
    }
  | {
      kind: "history";
      generation: string;
      entry_id: string;
      revision: number;
      block_index: number;
    };
