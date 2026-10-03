import type { LensDeliveryCoverage } from "../contracts/lens";

export type InputCoverage = "full" | "partial" | "unavailable" | "pending" | "unknown";

/** Prepared completeness measures additional loss relative to the acquired source, using the preparation receipt, not capture quality or submission. */
export function inputCoverage(
  delivery: LensDeliveryCoverage | undefined,
  unresolved: "pending" | "unknown",
): InputCoverage {
  if (!delivery) return unresolved;
  switch (delivery.mode) {
    case "unavailable":
      return "unavailable";
    case "text_only_partial":
      return "partial";
    case "complete":
      if (delivery.projection_has_loss === true) return "partial";
      return delivery.projection_has_loss === false ? "full" : "unknown";
    default:
      return assertNever(delivery.mode);
  }
}

function assertNever(mode: never): never {
  throw new Error(`Unsupported Agent delivery mode: ${mode}`);
}
