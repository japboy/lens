import { McpAppController, type McpAppsPort } from "adapter-mcp-apps-host";
import { prepareHtmlDocument } from "adapter-mcp-apps-view";
import { htmlMathInlineCss } from "virtual:lens-html-math-assets";
import { version } from "../../package.json";

export interface DesktopMcpAppsPort extends McpAppsPort {
  prepareMcpAppDocument(leaseId: string, document: string): Promise<void>;
  openHtmlPresentation(
    source: HtmlPresentationSource,
    hostOrigin: string,
  ): Promise<import("adapter-mcp-apps-host").McpAppLease>;
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

/** Desktop supplies product metadata and the built-in renderer; the Host is generic. */
export function createDesktopMcpAppController(
  port: DesktopMcpAppsPort,
  changed: () => void,
  bridgeFactory?: ConstructorParameters<typeof McpAppController>[3],
): McpAppController {
  return new McpAppController(
    port,
    changed,
    {
      hostInfo: { name: "Lens", version },
      frameTitle: "Interactive Interpretation",
      getHostContext: (lease, owner) => ({
        platform: "desktop",
        theme: owner.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light",
        displayMode: "inline",
        availableDisplayModes: ["inline"],
        ...(lease.document_url
          ? { lensDocumentUrl: lease.document_url, lensDocumentMode: lease.document_mode }
          : {}),
      }),
      prepareDocument: (lease) => {
        if (!lease.document_url) return;
        if (!lease.document_mode || typeof lease.input.html !== "string")
          throw new Error("Invalid HTML presentation document");
        return port.prepareMcpAppDocument(
          lease.id,
          prepareHtmlDocument(lease.input.html, lease.document_mode, htmlMathInlineCss),
        );
      },
    },
    bridgeFactory,
  );
}
