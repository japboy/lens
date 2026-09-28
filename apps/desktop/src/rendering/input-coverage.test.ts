import { describe, expect, it } from "vitest";
import { inputCoverage } from "./input-coverage";
import type { LensDeliveryCoverage } from "../types";

const complete: LensDeliveryCoverage = { mode: "complete", sources: [] };
const textOnly: LensDeliveryCoverage = { mode: "text_only_partial", sources: [] };
const unavailable: LensDeliveryCoverage = { mode: "unavailable", sources: [] };

describe("input coverage", () => {
  it("calls input full only when capture, projection, and current delivery are all complete", () => {
    expect(inputCoverage("full", true, false, complete, "pending")).toBe("full");
    expect(inputCoverage("full", true, true, complete, "pending")).toBe("partial");
    expect(inputCoverage("full", true, undefined, complete, "pending")).toBe("unknown");
    expect(inputCoverage("full", true, false, undefined, "pending")).toBe("pending");
    expect(inputCoverage("full", true, false, undefined, "unknown")).toBe("unknown");
  });

  it("keeps capture loss distinct from image-delivery loss and unavailable input", () => {
    expect(inputCoverage("partial", true, false, complete, "pending")).toBe("partial");
    expect(inputCoverage("full", true, false, textOnly, "pending")).toBe("partial");
    expect(inputCoverage("full", true, false, unavailable, "pending")).toBe("unavailable");
    expect(inputCoverage("unavailable", false, false, complete, "pending")).toBe("unavailable");
  });
});
