import { LitElement, html, nothing } from "lit";
import { customElement, property } from "lit/decorators.js";
import type { InputCoverage } from "../rendering/input-coverage";
import type {
  AgentRunState,
  LensContext,
  LensDeliveryCoverage,
  LensInput,
  ProjectionOmission,
} from "../types";

const PROJECTION_LOSS_LABELS = {
  token_budget: "Text budget",
  resource_budget: "Resource budget",
  unsupported_semantics: "Unsupported document semantics",
} as const satisfies Record<Exclude<ProjectionOmission["reason"], "application_chrome">, string>;

@customElement("lens-extraction-diagnostics")
export class LensExtractionDiagnostics extends LitElement {
  @property({ attribute: false })
  context: LensContext | undefined;

  @property({ attribute: false })
  agent: AgentRunState | undefined;

  @property({ attribute: false })
  delivery: LensDeliveryCoverage | undefined;

  @property({ attribute: false })
  inputStatus: InputCoverage = "pending";

  @property({ attribute: false })
  projectionHasLoss: boolean | undefined;

  @property({ attribute: false })
  input: LensInput | undefined;

  @property({ attribute: false })
  inputDetailsLoading = false;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    const context = this.context;
    if (!context) {
      return html`<section
        class="extraction-diagnostics"
        aria-labelledby="diagnostics-heading"
        aria-busy=${this.inputDetailsLoading ? "true" : "false"}
      >
        <header class="diagnostics-header">
          <h2 id="diagnostics-heading">Extraction diagnostics</h2>
        </header>
        ${this.inputDetailsLoading ? nothing : html`<p class="empty-state">No extraction diagnostics are available.</p>`}
        <div class="diagnostics-layout">
          ${this.renderProjectionOmissions()} ${this.renderAgentDelivery()}
        </div>
      </section>`;
    }
    const mediaMetrics = [
      [
        "AX image regions",
        context.media.filter((item) => item.scope === "ax_element_region").length,
      ],
      ["Window fallbacks", context.media.filter((item) => item.scope === "window_fallback").length],
      ["PNG bytes", context.media.reduce((total, item) => total + item.encoded_bytes, 0)],
      [
        "Images omitted during capture",
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
      <section
        class="extraction-diagnostics"
        aria-labelledby="diagnostics-heading"
        aria-busy=${this.inputDetailsLoading ? "true" : "false"}
      >
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
            <p>Captured image attachments; these counts do not describe what the Agent receives.</p>
            ${this.metricList(mediaMetrics)}
          </section>
          ${this.renderProjectionOmissions()} ${this.renderAgentDelivery(context)}
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

  private renderProjectionOmissions() {
    const projectionOmissions = this.input?.sources.flatMap((source) =>
      source.omissions.filter((omission) => omission.reason !== "application_chrome"),
    );
    return html`
      <section
        class="diagnostic-group"
        aria-labelledby="projection-loss-heading"
        aria-busy=${!this.input && this.inputDetailsLoading && this.projectionHasLoss !== false ? "true" : "false"}
      >
        <h2 id="projection-loss-heading">Projection omissions</h2>
        ${
          this.projectionHasLoss === false
            ? html`<p>No text, resource, or document content was omitted by projection.</p>`
            : projectionOmissions && projectionOmissions.length
              ? html`<p>${projectionOmissions.length} omission records in prepared input.</p>
                  <ul>
                    ${Object.entries(PROJECTION_LOSS_LABELS).map(([reason, label]) => {
                      const count = projectionOmissions.filter(
                        (omission) => omission.reason === reason,
                      ).length;
                      return count ? html`<li>${label}: ${count}</li>` : nothing;
                    })}
                  </ul>`
              : this.inputDetailsLoading
                ? nothing
                : html`<p>Details unavailable.</p>`
        }
      </section>
    `;
  }

  private renderAgentDelivery(context?: LensContext) {
    return html`
      <section class="diagnostic-group" aria-labelledby="agent-delivery-heading">
        <h2 id="agent-delivery-heading">Current prepared Agent input</h2>
        ${
          this.delivery
            ? html`
                <p>
                  Prepared input completeness relative to captured information. Capture quality is
                  shown separately; preparation does not confirm submission.
                </p>
                ${
                  this.delivery.mode === "unavailable"
                    ? html`<p>
                        No usable source text or images are available to this Agent. Capture the
                        sources again, or choose an image-capable Agent or a source with accessible
                        text.
                      </p>`
                    : nothing
                }
                ${this.metricList([
                  ["Prepared input coverage", this.inputStatus.toUpperCase()],
                  [
                    "Projection omissions",
                    this.projectionHasLoss === undefined
                      ? "Unknown"
                      : this.projectionHasLoss
                        ? "Present"
                        : "None",
                  ],
                  [
                    "Images omitted for Agent",
                    this.delivery.sources.reduce(
                      (total, source) => total + source.omitted_media.length,
                      0,
                    ),
                  ],
                  [
                    "Reason",
                    this.delivery.sources.some((source) => source.omitted_media.length)
                      ? "Image input not supported for this submission"
                      : "—",
                  ],
                ])}
                <ul>
                  ${this.delivery.sources.map((source, index) => {
                    const captured = context?.sources[index];
                    const label = captured
                      ? captured.source.window_title
                        ? `${captured.source.application} — ${captured.source.window_title}`
                        : captured.source.application
                      : `Source ${index + 1}`;
                    const status =
                      source.mode === "complete"
                        ? "Complete"
                        : source.mode === "unavailable"
                          ? "Unavailable: no usable source text"
                          : "Text only";
                    return html`<li>
                      ${label}: ${status}; ${source.omitted_media.length}
                      ${source.omitted_media.length === 1 ? "image" : "images"} omitted for Agent
                    </li>`;
                  })}
                </ul>
              `
            : html`<p>Agent input coverage is ${this.inputStatus} for the current projection.</p>`
        }
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
