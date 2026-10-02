import { LitElement, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import type { McpAppsServer } from "adapter-mcp-apps-host";
import type { McpServerToolCatalog } from "../types";
import "./lens-settings-help";
import { dispatchComponentEvent, SETTINGS_INTENT_EVENT } from "./events";
import "./lens-select";
import type { LensSelect } from "./lens-select";
const BUILTIN = "lens_rich_content";
@customElement("lens-mcp-app-settings")
export class LensMcpAppSettings extends LitElement {
  @property({ attribute: false }) servers: readonly McpAppsServer[] = [];
  @property({ attribute: false }) catalogs: readonly McpServerToolCatalog[] = [];
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
  acceptResetPresets(): void {
    this.servers = [];
    this.authoritativeServers = [];
    this.drafts = {};
    this.catalogs = [];
    this.editingId = BUILTIN;
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
    if (this.disabled || this.servers.some((server) => server.id === this.editingId)) return;
    const next = { ...this.drafts };
    delete next[this.editingId];
    this.drafts = next;
    this.editingId = BUILTIN;
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
          "Use a unique name of 1–64 ASCII letters, numbers, underscores or hyphens. lens_rich_content and lens_output are reserved.",
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
    const catalog =
      saved &&
      this.catalogs.find(
        (catalog) =>
          catalog.server.id === saved.id &&
          catalog.server.name === saved.name &&
          catalog.server.url === saved.url,
      );
    return html`<section class="settings-group" aria-labelledby="mcp-apps-heading">
      <div class="settings-heading-row">
        <h2 id="mcp-apps-heading">MCP</h2>
        <lens-settings-help
          .helpId=${"mcp-presets-help"}
          .label=${"About MCP presets"}
          .text=${"All registered MCP servers are available together. Changing this selector only chooses the preset to edit."}
        ></lens-settings-help>
      </div>
      <div class="settings-field">
        <lens-select
          label="MCP Preset"
          .value=${this.editingId}
          .disabled=${this.disabled}
          .options=${[
            { value: BUILTIN, label: "lens_rich_content · Built-in" },
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
      <div class="agent-actions preset-add-actions mcp-preset-actions">
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
            : draft
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
        draft
          ? html`<form @submit=${this.save}>
              <fieldset class="agent-preset-settings" ?disabled=${this.disabled}>
                <legend class="visually-hidden">MCP preset settings</legend>
                <div class="settings-field">
                  <div class="settings-label-help">
                    <label for="mcp-preset-name">Name</label>
                    <lens-settings-help
                      .helpId=${"mcp-name-help"}
                      .label=${"About MCP preset name"}
                      .text=${"Use a unique name of 1–64 ASCII letters, numbers, underscores or hyphens. Up to 16 external presets are supported. lens_rich_content and lens_output are reserved."}
                    ></lens-settings-help>
                  </div>
                  <input
                    id="mcp-preset-name"
                    class="external-executable-field"
                    data-lens-control="text-entry"
                    aria-label="MCP preset name"
                    aria-describedby="mcp-name-help"
                    type="text"
                    maxlength="64"
                    placeholder="Reference"
                    autocomplete="off"
                    .value=${draft.name}
                    @input=${(event: Event) => this.edit("name", (event.target as HTMLInputElement).value)}
                  />
                </div>
                <div class="settings-field">
                  <div class="settings-label-help">
                    <label for="mcp-preset-url">Streamable HTTP URL</label>
                    <lens-settings-help
                      .helpId=${"mcp-transport-help"}
                      .label=${"About MCP transport"}
                      .text=${"Streamable HTTP only. Authentication and stdio are not supported. URLs cannot contain credentials, a query or a fragment."}
                    ></lens-settings-help>
                  </div>
                  <input
                    id="mcp-preset-url"
                    class="external-executable-field"
                    data-lens-control="text-entry"
                    aria-label="Streamable HTTP URL"
                    aria-describedby="mcp-transport-help"
                    type="url"
                    placeholder="https://mcp.example.com/mcp"
                    autocomplete="off"
                    .value=${draft.url}
                    @input=${(event: Event) => this.edit("url", (event.target as HTMLInputElement).value)}
                  />
                </div>
                <div class="agent-actions mcp-preset-editor-actions">
                  <button
                    type="submit"
                    data-lens-button-role="primary"
                    aria-describedby="mcp-save-help"
                    ?disabled=${this.disabled || !dirty}
                  >
                    Save
                  </button>
                  <lens-settings-help
                    .helpId=${"mcp-save-help"}
                    .label=${"About saving MCP presets"}
                    .text=${"Saving or deleting a preset closes the current Agent connection. The updated MCP presets are used for the next Interpretation."}
                  ></lens-settings-help>
                </div>
              </fieldset>
            </form>`
          : nothing
      }
      ${this.error ? html`<p class="error" role="alert">${this.error}</p>` : nothing}
      <dl class="mcp-preset-summary">
        <div>
          <dt>MCP server</dt>
          <dd>
            ${
              draft && !saved
                ? html`<span>Not registered</span>`
                : html`<code>${saved?.name ?? BUILTIN}</code>`
            }
          </dd>
        </div>
        <div>
          <dt>Tools</dt>
          <dd>
            ${
              !draft
                ? html`<code>render_html</code>`
                : !saved
                  ? html`<span>Not registered</span>`
                  : !catalog
                    ? html`<span>Not loaded</span>`
                    : catalog.tools.length === 0
                      ? html`<span>No tools</span>`
                      : catalog.tools.map((tool) => html`<div><code>${tool}</code></div>`)
            }
          </dd>
        </div>
      </dl>
      <div class="mcp-preset-reset-actions">
        <button
          type="button"
          data-lens-button-role="destructive"
          ?disabled=${this.disabled}
          @click=${() => {
            if (!this.disabled)
              dispatchComponentEvent(this, SETTINGS_INTENT_EVENT, { type: "reset-mcp-presets" });
          }}
        >
          Reset Presets…
        </button>
      </div>
    </section>`;
  }
}
