import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { McpAppsServer } from "../mcp-apps/types";
import { dispatchComponentEvent, SETTINGS_INTENT_EVENT } from "./events";
import "./lens-select";
import type { LensSelect } from "./lens-select";
const BUILTIN = "lens_rich_html";
@customElement("lens-mcp-app-settings")
export class LensMcpAppSettings extends LitElement {
  @property({ attribute: false }) servers: readonly McpAppsServer[] = [];
  @property({ type: Boolean }) disabled = false;
  @state() private drafts: Record<string, McpAppsServer> = {};
  @state() private editingId = BUILTIN;
  @state() private error = "";
  private authoritativeServers: readonly McpAppsServer[] = [];
  protected createRenderRoot(): HTMLElement {
    return this;
  }
  protected willUpdate(changed: PropertyValues): void {
    if (
      !changed.has("servers") ||
      JSON.stringify(this.servers) === JSON.stringify(this.authoritativeServers)
    )
      return;
    const previous = new Map(this.authoritativeServers.map((server) => [server.id, server]));
    const next = { ...this.drafts };
    for (const old of this.authoritativeServers)
      if (!this.servers.some((server) => server.id === old.id)) delete next[old.id];
    for (const server of this.servers)
      if (
        !previous.has(server.id) ||
        JSON.stringify(previous.get(server.id)) !== JSON.stringify(server)
      )
        next[server.id] = { ...server };
    this.authoritativeServers = this.servers.map((server) => ({ ...server }));
    this.drafts = next;
    if (this.editingId !== BUILTIN && !next[this.editingId]) this.editingId = BUILTIN;
    this.error = "";
  }
  private edit(field: "name" | "url", value: string): void {
    if (this.disabled || !this.drafts[this.editingId]) return;
    this.drafts = {
      ...this.drafts,
      [this.editingId]: { ...this.drafts[this.editingId]!, [field]: value },
    };
    this.error = "";
  }
  private add(): void {
    if (this.disabled || Object.keys(this.drafts).length >= 16) return;
    const id = crypto.randomUUID();
    this.drafts = { ...this.drafts, [id]: { id, name: "", url: "" } };
    this.editingId = id;
    this.error = "";
  }
  private discard(): void {
    if (this.disabled) return;
    const saved = this.servers.find((server) => server.id === this.editingId);
    if (saved) this.drafts = { ...this.drafts, [saved.id]: { ...saved } };
    else {
      const next = { ...this.drafts };
      delete next[this.editingId];
      this.drafts = next;
      this.editingId = BUILTIN;
    }
    this.error = "";
  }
  private publish(servers: McpAppsServer[]): void {
    if (servers.length > 16) throw new Error("At most 16 external MCP presets are supported.");
    const ids = new Set<string>(),
      names = new Set<string>();
    for (const server of servers) {
      if (
        !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(server.id) ||
        server.id === "00000000-0000-0000-0000-000000000000" ||
        ids.has(server.id)
      )
        throw new Error("MCP preset IDs must be valid and unique.");
      ids.add(server.id);
      if (
        !/^[A-Za-z0-9_-]{1,64}$/.test(server.name) ||
        [BUILTIN, "lens_output"].includes(server.name) ||
        names.has(server.name)
      )
        throw new Error(
          "Use a unique name of 1–64 ASCII letters, numbers, underscores or hyphens. lens_rich_html and lens_output are reserved.",
        );
      names.add(server.name);
      let url: URL;
      try {
        url = new URL(server.url);
      } catch {
        throw new Error("Enter an absolute HTTP or HTTPS MCP endpoint.");
      }
      if (
        !["http:", "https:"].includes(url.protocol) ||
        !url.hostname ||
        url.username ||
        url.password ||
        server.url.includes("?") ||
        server.url.includes("#")
      )
        throw new Error(
          "Enter an HTTP or HTTPS MCP endpoint without credentials, query or fragment.",
        );
    }
    dispatchComponentEvent(this, SETTINGS_INTENT_EVENT, { type: "set-mcp-apps-servers", servers });
  }
  private save(event: SubmitEvent): void {
    event.preventDefault();
    if (this.disabled || this.editingId === BUILTIN) return;
    const draft = this.drafts[this.editingId];
    if (!draft) return;
    const normalized = { ...draft, name: draft.name.trim(), url: draft.url.trim() };
    const servers = this.servers.some((server) => server.id === draft.id)
      ? this.servers.map((server) => (server.id === draft.id ? normalized : { ...server }))
      : [...this.servers.map((server) => ({ ...server })), normalized];
    this.error = "";
    try {
      this.publish(servers);
    } catch (error) {
      this.error = error instanceof Error ? error.message : String(error);
    }
  }
  private deletePreset(): void {
    if (
      this.disabled ||
      this.editingId === BUILTIN ||
      !this.servers.some((server) => server.id === this.editingId)
    )
      return;
    this.error = "";
    try {
      this.publish(
        this.servers
          .filter((server) => server.id !== this.editingId)
          .map((server) => ({ ...server })),
      );
    } catch (error) {
      this.error = error instanceof Error ? error.message : String(error);
    }
  }
  protected render() {
    const draft = this.drafts[this.editingId],
      saved = this.servers.find((server) => server.id === this.editingId);
    const dirty = Boolean(draft && JSON.stringify(draft) !== JSON.stringify(saved));
    return html`<section class="settings-group" aria-labelledby="mcp-apps-heading">
      <h2 id="mcp-apps-heading">MCP</h2>
      <p class="help">
        Edit connection presets here. All registered MCP servers remain available together; this
        selection only chooses the preset to edit.
      </p>
      <div class="settings-field">
        <lens-select
          label="MCP preset to edit"
          .value=${this.editingId}
          .disabled=${this.disabled}
          .options=${[
            { value: BUILTIN, label: "Lens HTML · Built-in" },
            ...Object.values(this.drafts).map((server) => ({
              value: server.id,
              label: `${server.name || "New preset"}${this.servers.some((saved) => saved.id === server.id) ? "" : " (unsaved)"}`,
            })),
          ]}
          @change=${(event: Event) => {
            this.editingId = (event.target as LensSelect).value;
            this.error = "";
          }}
        ></lens-select>
      </div>
      <div class="agent-actions preset-add-actions">
        <button
          type="button"
          data-lens-button-role="normal"
          ?disabled=${this.disabled || Object.keys(this.drafts).length >= 16}
          @click=${() => this.add()}
        >
          Add Preset
        </button>
        ${
          saved
            ? html`<button
                type="button"
                data-lens-button-role="destructive"
                ?disabled=${this.disabled}
                @click=${() => this.deletePreset()}
              >
                Delete Preset
              </button>`
            : nothing
        }
        ${
          draft && !saved
            ? html`<button
                type="button"
                data-lens-button-role="cancel"
                ?disabled=${this.disabled}
                @click=${() => this.discard()}
              >
                Discard Draft
              </button>`
            : nothing
        }
      </div>
      ${
        this.editingId === BUILTIN
          ? html`
              <p class="help">
                Lens HTML is always available. Its render_html tool displays rich, interactive HTML
                in Lens. This built-in preset cannot be edited or deleted.
              </p>
              <dl>
                <dt>MCP server</dt>
                <dd>lens_rich_html</dd>
                <dt>Tool</dt>
                <dd>render_html</dd>
              </dl>
            `
          : draft
            ? html`<form @submit=${this.save}>
                <fieldset class="agent-preset-settings" ?disabled=${this.disabled}>
                  <legend class="visually-hidden">MCP preset settings</legend>
                  <label class="settings-field"
                    ><span>Name</span
                    ><input
                      class="external-executable-field"
                      data-lens-control="text-entry"
                      aria-label="MCP preset name"
                      type="text"
                      maxlength="64"
                      placeholder="Reference"
                      .value=${draft.name}
                      @input=${(event: Event) => this.edit("name", (event.target as HTMLInputElement).value)}
                  /></label>
                  <label class="settings-field"
                    ><span>MCP Endpoint</span
                    ><input
                      class="external-executable-field"
                      data-lens-control="text-entry"
                      aria-label="MCP endpoint"
                      type="url"
                      placeholder="https://mcp.example.com/mcp"
                      .value=${draft.url}
                      @input=${(event: Event) => this.edit("url", (event.target as HTMLInputElement).value)}
                  /></label>
                  <p class="help">
                    Use a unique name of 1–64 ASCII letters, numbers, underscores or hyphens. Up to
                    16 external presets are supported.
                  </p>
                  <p class="help">
                    Streamable HTTP only. Authentication and stdio are not supported. Endpoints
                    cannot contain credentials, a query or a fragment.
                  </p>
                  <div class="agent-actions">
                    ${
                      saved
                        ? html`<button
                            type="button"
                            data-lens-button-role="cancel"
                            ?disabled=${this.disabled || !dirty}
                            @click=${() => this.discard()}
                          >
                            Discard Draft
                          </button>`
                        : nothing
                    }
                    <button
                      type="submit"
                      data-lens-button-role="primary"
                      ?disabled=${this.disabled || !dirty}
                    >
                      Save Preset
                    </button>
                  </div>
                </fieldset>
              </form>`
            : nothing
      }
      ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
      <p class="help">
        Saving or deleting a preset closes the current Agent connection. The full registry is used
        when you start the next Interpretation.
      </p>
    </section>`;
  }
}
