export type Resource<T> =
  | { stage: "loading" }
  | { stage: "ready"; value: T }
  | { stage: "failed"; message: string };
export type ResourceState =
  | { stage: "idle" | "loading" | "ready" }
  | { stage: "failed"; message: string };
export type SnapshotConnectionState =
  | { stage: "subscribing" }
  | { stage: "loading" }
  | { stage: "ready" }
  | { stage: "failed"; message: string };
export type AccessibilityPermissionState =
  | { stage: "inactive" }
  | { stage: "checking" }
  | { stage: "ready"; capabilities: readonly CapabilityInspection[] }
  | { stage: "failed"; message: string };

export const SOURCE_CAPABILITIES = [
  "target_selection",
  "structured_extraction",
  "image_capture",
  "observation",
] as const;
export type SourceCapability = (typeof SOURCE_CAPABILITIES)[number];
export type AccessStatus =
  | "available"
  | "permission_required"
  | "restricted"
  | "unsupported"
  | "failed";
export interface CapabilityInspection {
  capability: SourceCapability;
  status: AccessStatus;
  request_action: "request_permission" | null;
  diagnostics: readonly string[];
}

/** Reject unknown native statuses and malformed requestability at the IPC boundary. */
export function parseCapabilityInspection(value: unknown): CapabilityInspection {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid capability inspection");
  const wire = value as Record<string, unknown>;
  if (
    Object.keys(wire).length !== 4 ||
    !SOURCE_CAPABILITIES.some((capability) => capability === wire.capability) ||
    typeof wire.status !== "string" ||
    !["available", "permission_required", "restricted", "unsupported", "failed"].includes(
      wire.status,
    ) ||
    !(
      wire.request_action === null ||
      (wire.request_action === "request_permission" && wire.status === "permission_required")
    ) ||
    !Array.isArray(wire.diagnostics) ||
    !wire.diagnostics.every((detail) => typeof detail === "string")
  ) {
    throw new Error("Invalid capability inspection");
  }
  return wire as unknown as CapabilityInspection;
}

export function parseCapabilityInspections(value: unknown): CapabilityInspection[] {
  if (!Array.isArray(value) || value.length !== SOURCE_CAPABILITIES.length)
    throw new Error("Incomplete capability inspection");
  const reports = value.map(parseCapabilityInspection);
  return SOURCE_CAPABILITIES.map((capability) => {
    const matches = reports.filter((report) => report.capability === capability);
    if (matches.length !== 1) throw new Error("Duplicate or missing capability inspection");
    return matches[0]!;
  });
}
export type SnapshotStatus =
  | { stage: "loading" }
  | { stage: "ready" }
  | { stage: "failed"; message: string };
