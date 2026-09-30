import { LitElement, html, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { repeat } from "lit/directives/repeat.js";
import type { McpAppsServer } from "../mcp-apps/types";
import { dispatchComponentEvent, SETTINGS_INTENT_EVENT } from "./events";

@customElement("lens-mcp-app-settings")
export class LensMcpAppSettings extends LitElement {
  @property({ attribute: false }) servers: readonly McpAppsServer[] = [];
  @property({ type: Boolean }) disabled = false;
  @state() private drafts: McpAppsServer[] = [];
  @state() private error = "";
  private authoritativeServers = "";
  protected createRenderRoot(): HTMLElement {
    return this;
  }
  protected willUpdate(changed: PropertyValues): void {
    if (!changed.has("servers")) return;
    const authoritativeServers = JSON.stringify(this.servers);
    if (authoritativeServers === this.authoritativeServers) return;
    this.authoritativeServers = authoritativeServers;
    this.drafts = this.servers.map((server) => ({ ...server }));
  }
  private edit(id: string, field: "name" | "url", value: string): void {
    this.drafts = this.drafts.map((server) =>
      server.id === id ? { ...server, [field]: value } : server,
    );
  }
  private save(): void {
    this.error = "";
    try {
      for (const server of this.drafts) {
        if (!server.name.trim()) throw new Error("Enter a server name.");
        const url = new URL(server.url);
        if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.hash)
          throw new Error("Enter an HTTP or HTTPS MCP endpoint without credentials or a fragment.");
      }
      dispatchComponentEvent(this, SETTINGS_INTENT_EVENT, {
        type: "set-mcp-apps-servers",
        servers: this.drafts.map((server) => ({
          ...server,
          name: server.name.trim(),
          url: server.url.trim(),
        })),
      });
    } catch (error) {
      this.error = String(error instanceof Error ? error.message : error);
    }
  }
  protected render() {
    return html`<section class="settings-group" aria-labelledby="mcp-apps-heading">
      <h2 id="mcp-apps-heading">MCP Apps</h2>
      <p class="help">
        Add MCP servers whose tools and Apps you want to make available to the Agent. Lens also
        includes a rich HTML App.
      </p>
      <p class="help">
        Streamable HTTP endpoints are supported. Authentication and stdio servers are not supported
        here.
      </p>
      ${repeat(
        this.drafts,
        (server) => server.id,
        (server) =>
          html`<div class="mcp-app-server">
            <label
              >Server Name<input
                data-lens-control="text-entry"
                type="text"
                .value=${server.name}
                ?disabled=${this.disabled}
                @input=${(event: Event) => this.edit(server.id, "name", (event.target as HTMLInputElement).value)} /></label
            ><label
              >MCP Endpoint<input
                data-lens-control="text-entry"
                type="url"
                .value=${server.url}
                placeholder="https://example.com/mcp"
                ?disabled=${this.disabled}
                @input=${(event: Event) => this.edit(server.id, "url", (event.target as HTMLInputElement).value)} /></label
            ><button
              type="button"
              data-lens-button-role="destructive"
              ?disabled=${this.disabled}
              @click=${() => {
                this.drafts = this.drafts.filter((item) => item.id !== server.id);
              }}
            >
              Remove Server
            </button>
          </div>`,
      )}
      ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : ""}
      <div class="settings-actions">
        <button
          type="button"
          data-lens-button-role="normal"
          ?disabled=${this.disabled || this.drafts.length >= 16}
          @click=${() => {
            this.drafts = [...this.drafts, { id: crypto.randomUUID(), name: "", url: "" }];
          }}
        >
          Add Server</button
        ><button
          type="button"
          data-lens-button-role="primary"
          ?disabled=${this.disabled}
          @click=${() => this.save()}
        >
          Save Servers
        </button>
      </div>
      <p class="help">
        Saving server changes closes the current Agent connection. The new configuration is used
        when you start the next Interpretation.
      </p>
    </section>`;
  }
}
