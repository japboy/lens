import { LitElement, html, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import type { AgentRunState, LensContext } from "../types";

@customElement("lens-extraction-diagnostics")
export class LensExtractionDiagnostics extends LitElement {
  @property({ attribute: false })
  context: LensContext | undefined;

  @property({ attribute: false })
  agent: AgentRunState | undefined;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    const context = this.context;
    if (!context) return nothing;
    const mediaMetrics = [
      [
        "AX image regions",
        context.media.filter((item) => item.scope === "ax_element_region").length,
      ],
      ["Window fallbacks", context.media.filter((item) => item.scope === "window_fallback").length],
      ["PNG bytes", context.media.reduce((total, item) => total + item.encoded_bytes, 0)],
      [
        "Omitted images",
        context.media_omissions.reduce((total, item) => total + item.omitted_count, 0),
      ],
    ] as const;
    const agentMetrics = this.agent
      ? ([
          ["ACP Agent", `${this.agent.adapter_name} ${this.agent.adapter_version}`],
          ["Session updates", this.agent.received_updates],
          ["Stop reason", this.agent.stop_reason ?? "—"],
        ] as const)
      : [];
    const diagnosticCount = context.diagnostics.length;

    return html`
      <section class="extraction-diagnostics" aria-labelledby="diagnostics-heading">
        <header class="diagnostics-header">
          <h2 id="diagnostics-heading">Extraction diagnostics</h2>
          <span class="diagnostic-count">
            ${
              diagnosticCount === 0
                ? "No messages"
                : `${diagnosticCount} ${diagnosticCount === 1 ? "message" : "messages"}`
            }
          </span>
        </header>
        <div class="diagnostics-layout">
          ${context.sources.map((source, index) => {
            const accessibility = source.capture;
            const metrics = [
              ["Quality", source.quality],
              ["Visited nodes", accessibility.metrics.visited_nodes],
              ["UTF-8 bytes", accessibility.metrics.text_bytes],
              ["Off-window text nodes", accessibility.metrics.offscreen_text_nodes],
              ["Virtualization signals", accessibility.metrics.virtualization_signals],
              ["Child read errors", accessibility.metrics.children_read_errors],
              ["URI resource references", accessibility.metrics.resource_ref_count],
              ["URI UTF-8 bytes", accessibility.metrics.resource_uri_bytes],
              ["Omitted URI references", accessibility.metrics.omitted_resource_refs],
              ["URI read errors", accessibility.metrics.resource_read_errors],
              ["Nodes truncated", accessibility.metrics.truncated_nodes ? "Yes" : "No"],
              ["Text truncated", accessibility.metrics.truncated_text ? "Yes" : "No"],
            ] as const;
            const headingId = `accessibility-metrics-heading-${index}`;
            const sourceLabel = source.source.window_title
              ? `${source.source.application} — ${source.source.window_title}`
              : source.source.application;
            return html`
              <section class="diagnostic-group" aria-labelledby=${headingId}>
                <h2 id=${headingId}>Accessibility — ${sourceLabel}</h2>
                ${this.metricList(metrics)}
              </section>
            `;
          })}
          <section class="diagnostic-group" aria-labelledby="media-metrics-heading">
            <h2 id="media-metrics-heading">AX-linked images</h2>
            ${this.metricList(mediaMetrics)}
          </section>
          ${
            agentMetrics.length
              ? html`<section class="diagnostic-group" aria-labelledby="agent-metrics-heading">
                  <h2 id="agent-metrics-heading">Agent session</h2>
                  ${this.metricList(agentMetrics)}
                </section>`
              : nothing
          }
          <section
            class="diagnostic-group diagnostic-messages"
            aria-labelledby="diagnostic-messages-heading"
          >
            <h2 id="diagnostic-messages-heading">Messages</h2>
            ${
              diagnosticCount
                ? html`<ul>
                    ${context.diagnostics.map((item) => html`<li>${item}</li>`)}
                  </ul>`
                : html`<p>No extraction warnings or errors.</p>`
            }
          </section>
        </div>
      </section>
    `;
  }

  private metricList(metrics: ReadonlyArray<readonly [string, string | number | boolean]>) {
    return html`<dl class="metrics">
      ${metrics.map(
        ([label, value]) =>
          html`<div>
            <dt>${label}</dt>
            <dd>${value}</dd>
          </div>`,
      )}
    </dl>`;
  }
}
