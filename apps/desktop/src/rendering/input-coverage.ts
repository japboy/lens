import type { LensDeliveryCoverage } from "../types";

export type InputCoverage = "full" | "partial" | "unavailable" | "pending" | "unknown";

/** Prepared completeness measures additional loss relative to the acquired source, not capture quality or submission. */
export function inputCoverage(
  hasInput: boolean | undefined,
  projectionHasLoss: boolean | undefined,
  delivery: LensDeliveryCoverage | undefined,
  unresolved: "pending" | "unknown",
): InputCoverage {
  if (!delivery) return unresolved;
  switch (delivery.mode) {
    case "unavailable":
      return "unavailable";
    case "text_only_partial":
      return hasInput === false ? "unavailable" : hasInput === true ? "partial" : "unknown";
    case "complete":
      if (hasInput === false) return "unavailable";
      if (hasInput !== true) return "unknown";
      if (projectionHasLoss === true) return "partial";
      return projectionHasLoss === false ? "full" : "unknown";
    default:
      return assertNever(delivery.mode);
  }
}

/** Replay uses the immutable preparation receipt; legacy missing metadata remains unknown. */
export function responseInputCoverage(delivery: LensDeliveryCoverage | undefined): InputCoverage {
  return inputCoverage(
    delivery ? delivery.mode !== "unavailable" : undefined,
    delivery?.projection_has_loss,
    delivery,
    "unknown",
  );
}

function assertNever(mode: never): never {
  throw new Error(`Unsupported Agent delivery mode: ${mode}`);
}
