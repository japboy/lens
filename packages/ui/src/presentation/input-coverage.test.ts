import { describe, expect, it } from "vitest";
import { inputCoverage } from "./input-coverage";
import type { LensDeliveryCoverage } from "../contracts/lens";

const complete: LensDeliveryCoverage = {
  mode: "complete",
  sources: [],
  projection_has_loss: false,
};

describe("prepared input coverage", () => {
  it("uses the receipt's completeness and availability, including legacy missing evidence", () => {
    expect(inputCoverage(complete, "unknown")).toBe("full");
    expect(inputCoverage({ ...complete, projection_has_loss: true }, "unknown")).toBe("partial");
    expect(inputCoverage({ ...complete, projection_has_loss: undefined }, "unknown")).toBe(
      "unknown",
    );
    expect(inputCoverage({ mode: "text_only_partial", sources: [] }, "unknown")).toBe("partial");
    expect(
      inputCoverage({ mode: "unavailable", sources: [], projection_has_loss: true }, "unknown"),
    ).toBe("unavailable");
  });

  it("keeps missing preparation pending or unknown according to source currentness", () => {
    expect(inputCoverage(undefined, "pending")).toBe("pending");
    expect(inputCoverage(undefined, "unknown")).toBe("unknown");
  });
});
