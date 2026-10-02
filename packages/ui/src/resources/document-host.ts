import { McpAppController } from "adapter-mcp-apps-host";
import type { UiMcpAppsPort } from "../contracts/document-host";
export function createUiMcpAppController(
  port: UiMcpAppsPort,
  changed: () => void,
): McpAppController {
  return new McpAppController(port, changed, port.presentation);
}
