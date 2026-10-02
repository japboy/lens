import { describe, it, expect } from "vitest";
import { shouldApplySnapshot } from "./snapshot-admission";
describe("snapshot admission", () => {
  it("accepts only a strictly newer finite application snapshot", () => {
    expect(shouldApplySnapshot(-1, 0)).toBe(true);
    expect(shouldApplySnapshot(7, 8)).toBe(true);
    expect(shouldApplySnapshot(7, 7)).toBe(false);
    expect(shouldApplySnapshot(7, 6)).toBe(false);
    expect(shouldApplySnapshot(7, Number.NaN)).toBe(false);
    expect(shouldApplySnapshot(7, 7.5)).toBe(false);
  });
});
