import { McpAppController, type McpAppsPort } from "adapter-mcp-apps-host";
import { prepareRichHtmlDocument } from "adapter-mcp-apps-view";
import { htmlMathInlineCss } from "virtual:lens-html-math-assets";
import { version } from "../../package.json";

export interface DesktopMcpAppsPort extends McpAppsPort {
  prepareMcpAppDocument(leaseId: string, document: string): Promise<void>;
}

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
        ...(lease.document_url ? { lensDocumentUrl: lease.document_url } : {}),
      }),
      prepareDocument: (lease, descriptor) => {
        if (!lease.document_url) return;
        if (descriptor.server_id !== "lens_rich_content" || typeof lease.input.html !== "string")
          throw new Error("Invalid built-in App document");
        return port.prepareMcpAppDocument(
          lease.id,
          prepareRichHtmlDocument(lease.input.html, htmlMathInlineCss),
        );
      },
    },
    bridgeFactory,
  );
}
