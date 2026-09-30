import { describe, expect, it } from "vitest";
import { inputCoverage, responseInputCoverage } from "./input-coverage";
import type { LensDeliveryCoverage } from "../types";

const complete: LensDeliveryCoverage = { mode: "complete", sources: [] };
const textOnly: LensDeliveryCoverage = { mode: "text_only_partial", sources: [] };
const unavailable: LensDeliveryCoverage = { mode: "unavailable", sources: [] };

describe("prepared input coverage", () => {
  it("measures only additional projection and delivery loss", () => {
    expect(inputCoverage(true, false, complete, "pending")).toBe("full");
    expect(inputCoverage(true, true, complete, "pending")).toBe("partial");
    expect(inputCoverage(true, false, textOnly, "pending")).toBe("partial");
    expect(inputCoverage(true, true, textOnly, "pending")).toBe("partial");
    expect(inputCoverage(true, false, unavailable, "pending")).toBe("unavailable");
    expect(inputCoverage(false, false, complete, "pending")).toBe("unavailable");
    expect(inputCoverage(false, false, textOnly, "pending")).toBe("unavailable");
  });

  it("requires evidence and preserves unresolved currentness", () => {
    expect(inputCoverage(true, undefined, complete, "pending")).toBe("unknown");
    expect(inputCoverage(undefined, false, complete, "pending")).toBe("unknown");
    expect(inputCoverage(undefined, false, textOnly, "pending")).toBe("unknown");
    expect(inputCoverage(true, false, undefined, "pending")).toBe("pending");
    expect(inputCoverage(true, false, undefined, "unknown")).toBe("unknown");
  });

  it("uses only the historical receipt and keeps legacy missing evidence unknown", () => {
    expect(responseInputCoverage({ ...complete, projection_has_loss: false })).toBe("full");
    expect(responseInputCoverage({ ...complete, projection_has_loss: true })).toBe("partial");
    expect(responseInputCoverage(complete)).toBe("unknown");
    expect(responseInputCoverage(undefined)).toBe("unknown");
    expect(responseInputCoverage(textOnly)).toBe("partial");
    expect(responseInputCoverage(unavailable)).toBe("unavailable");
  });
});
