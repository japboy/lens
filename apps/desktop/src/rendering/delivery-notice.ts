import { html, nothing } from "lit";
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
