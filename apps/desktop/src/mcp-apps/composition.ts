import { McpAppController } from "adapter-mcp-apps-host";
import { prepareHtmlDocument } from "adapter-mcp-apps-view";
import { htmlMathInlineCss } from "virtual:lens-html-math-assets";
import { version } from "../../package.json";

import type { UiMcpAppsPort } from "ui/contracts/document-host";
export interface DesktopMcpAppsPort extends Omit<UiMcpAppsPort, "presentation"> {
  prepareMcpAppDocument(leaseId: string, document: string): Promise<void>;
}
export function desktopMcpAppHostOptions(
  port: DesktopMcpAppsPort,
): import("adapter-mcp-apps-host").McpAppHostOptions {
  return {
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
  };
}
export function toUiMcpAppsPort(port: DesktopMcpAppsPort): UiMcpAppsPort {
  return { ...port, presentation: desktopMcpAppHostOptions(port) };
}
/** Native preparation and product metadata are application-owned. */
export function createDesktopMcpAppController(
  port: DesktopMcpAppsPort,
  changed: () => void,
  bridgeFactory?: ConstructorParameters<typeof McpAppController>[3],
): McpAppController {
  return new McpAppController(port, changed, desktopMcpAppHostOptions(port), bridgeFactory);
}
