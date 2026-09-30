import { html, nothing } from "lit";
import { keyed } from "lit/directives/keyed.js";
import { repeat } from "lit/directives/repeat.js";
import type { ResponseHistoryPresentation } from "../application/response-history-controller";
import { inputCoverage } from "./input-coverage";
import type { LensDeliveryCoverage } from "../types";

/** Coverage belongs to its submitted response, independently of current capture quality. */
export function renderResponseDeliveryNotice(delivery: LensDeliveryCoverage | undefined) {
  if (!delivery || delivery.mode === "complete") return nothing;
  const unavailable = delivery.mode === "unavailable";
  return html`<aside class="lens-delivery-notice" data-delivery-mode=${delivery.mode}>
    <strong
      >${unavailable ? "Input unavailable to this agent" : "This response used text only"}</strong
    >
    <p>
      ${
        unavailable
          ? "No usable source text or images were available to this agent. Capture the sources again, or choose an image-capable agent or a source with accessible text."
          : "Only accessible text was sent; captured images were omitted."
      }
    </p>
    <ul>
      ${delivery.sources
        .filter((source) => source.mode !== "complete")
        .map((source) => {
          const index = /^source-(\d+)$/.exec(source.source_id)?.[1];
          const label = index === undefined ? source.source_id : `Source ${Number(index) + 1}`;
          return html`<li>
            ${label}:
            ${source.mode === "unavailable" ? "not interpreted (no usable text)" : "text only; images omitted"}
          </li>`;
        })}
    </ul>
  </aside>`;
}

/** Live Diagnostics owns immutable response receipts without exposing them in saved sessions. */
export function renderResponseInputDiagnostics(history: ResponseHistoryPresentation | undefined) {
  if (!history) return nothing;
  const affected = history.responses.filter(
    (response) => inputCoverage(response.delivery, "unknown") !== "full",
  );
  return keyed(
    history.scopeId,
    html`<section class="response-input-diagnostics" aria-labelledby="response-input-heading">
      <h2 id="response-input-heading">Response input history</h2>
      ${
        affected.length
          ? repeat(
              affected,
              (response) => response.id,
              (response) => html`<details
                class="lens-response-coverage"
                aria-label=${`Response ${response.sequence} input coverage`}
              >
                <summary>
                  Response ${response.sequence} · Agent input:
                  ${inputCoverage(response.delivery, "unknown").toUpperCase()}
                </summary>
                ${renderResponseDeliveryNotice(response.delivery)}
                ${response.delivery?.projection_has_loss === true ? html`<p>Text, resource, or document content was omitted when preparing this response.</p>` : nothing}
                ${inputCoverage(response.delivery, "unknown") === "unknown" ? html`<p>Preparation information is unavailable for this response.</p>` : nothing}
              </details>`,
            )
          : html`<p>
              ${history.responses.length ? "All recorded responses have complete prepared input." : "No response input diagnostics are available."}
            </p>`
      }
    </section>`,
  );
}
