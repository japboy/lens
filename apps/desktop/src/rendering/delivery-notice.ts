import { html, nothing } from "lit";
import type { LensDeliveryCoverage } from "../types";

/** Coverage belongs to its submitted response, independently of current capture quality. */
export function renderDeliveryNotice(
  delivery: LensDeliveryCoverage | undefined,
  scope: "current" | "response",
  sourceLabels: readonly string[] = [],
) {
  if (!delivery || delivery.mode === "complete") return nothing;
  const unavailable = delivery.mode === "unavailable";
  const title = unavailable
    ? "Input unavailable to this agent"
    : scope === "response"
      ? "This response used text only"
      : "Text-only input";
  return html`<aside class="lens-delivery-notice" role="status" data-delivery-mode=${delivery.mode}>
    <strong>${title}</strong>
    <p>
      ${
        unavailable
          ? "No usable source text can be sent. Choose an image-capable agent or a source with accessible text."
          : scope === "response"
            ? "Only accessible text was sent; captured images were omitted."
            : "Only accessible text will be used; captured images are omitted for this agent."
      }
    </p>
    <ul>
      ${delivery.sources
        .filter((source) => source.mode !== "complete")
        .map((source) => {
          const index = /^source-(\d+)$/.exec(source.source_id)?.[1];
          const label =
            index === undefined
              ? source.source_id
              : sourceLabels[Number(index)] || `Source ${Number(index) + 1}`;
          return html`<li>
            ${label}:
            ${source.mode === "unavailable" ? "not interpreted (no usable text)" : "text only; images omitted"}
          </li>`;
        })}
    </ul>
  </aside>`;
}
