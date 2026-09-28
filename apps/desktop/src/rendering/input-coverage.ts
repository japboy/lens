import type { ExtractionQuality, LensDeliveryCoverage } from "../types";

export type InputCoverage = "full" | "partial" | "unavailable" | "pending" | "unknown";

/** A complete input requires capture, projection, and Agent delivery to be accounted for. */
export function inputCoverage(
  capture: ExtractionQuality | null | undefined,
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
      return hasInput === false || capture === "unavailable" ? "unavailable" : "partial";
    case "complete":
      if (hasInput === false || capture === "unavailable") return "unavailable";
      if (capture === "partial" || projectionHasLoss === true) return "partial";
      return hasInput === true && capture === "full" && projectionHasLoss === false
        ? "full"
        : "unknown";
    default:
      return assertNever(delivery.mode);
  }
}

function assertNever(mode: never): never {
  throw new Error(`Unsupported Agent delivery mode: ${mode}`);
}
