// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { LensMcpAppSettings } from "./lens-mcp-app-settings";
import { SETTINGS_INTENT_EVENT } from "./events";

afterEach(() => document.body.replaceChildren());
describe("MCP Apps settings", () => {
  it("retains an unsaved endpoint during equal authoritative snapshots and emits a reviewed configuration", async () => {
    const element = new LensMcpAppSettings();
    element.servers = [{ id: "server", name: "Original", url: "https://example.com/mcp" }];
    document.body.append(element);
    await element.updateComplete;
    const inputs = element.querySelectorAll("input");
    inputs[1]!.value = "https://example.com/new-mcp";
    inputs[1]!.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;
    element.servers = element.servers.map((server) => ({ ...server }));
    await element.updateComplete;
    expect(element.querySelectorAll("input")[1]!.value).toBe("https://example.com/new-mcp");
    let intent: unknown;
    element.addEventListener(SETTINGS_INTENT_EVENT, (event) => {
      intent = (event as CustomEvent).detail;
    });
    [...element.querySelectorAll("button")]
      .find((button) => button.textContent?.includes("Save Servers"))!
      .click();
    expect(intent).toEqual({
      type: "set-mcp-apps-servers",
      servers: [{ id: "server", name: "Original", url: "https://example.com/new-mcp" }],
    });
    element.servers = [{ id: "server", name: "Confirmed", url: "https://example.com/saved" }];
    await element.updateComplete;
    expect(element.querySelectorAll("input")[1]!.value).toBe("https://example.com/saved");
  });
});
