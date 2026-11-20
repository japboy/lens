import { describe, expect, it } from "vitest";
import {
  parseCapabilityInspection,
  parseCapabilityInspections,
  SOURCE_CAPABILITIES,
} from "./resource-state";

const report = () => ({
  capability: "structured_extraction",
  status: "permission_required",
  request_action: "request_permission",
  diagnostics: [],
});
describe("capability access ingress", () => {
  it("requires finite literal statuses and real requestability without coercion", () => {
    expect(parseCapabilityInspection(report()).status).toBe("permission_required");
    for (const invalid of [undefined, null, 1, ["available"], {}, "future_status"]) {
      expect(() =>
        parseCapabilityInspection({ ...report(), status: invalid, request_action: null }),
      ).toThrow(/capability inspection/);
    }
    for (const status of ["available", "restricted", "unsupported", "failed"]) {
      expect(() => parseCapabilityInspection({ ...report(), status })).toThrow(
        /capability inspection/,
      );
      expect(parseCapabilityInspection({ ...report(), status, request_action: null }).status).toBe(
        status,
      );
    }
    expect(() =>
      parseCapabilityInspection({ ...report(), request_action: "grant_access" }),
    ).toThrow(/capability inspection/);
    expect(() => parseCapabilityInspection({ ...report(), diagnostics: [123] })).toThrow(
      /capability inspection/,
    );
  });
  it("requires exactly one report for every capability", () => {
    const reports = SOURCE_CAPABILITIES.map((capability) => ({ ...report(), capability }));
    expect(
      parseCapabilityInspections([...reports].reverse()).map((report) => report.capability),
    ).toEqual(SOURCE_CAPABILITIES);
    expect(() => parseCapabilityInspections(reports.slice(1))).toThrow(/capability inspection/);
    expect(() =>
      parseCapabilityInspections([reports[0], reports[0], reports[2], reports[3]]),
    ).toThrow(/capability inspection/);
  });
});
