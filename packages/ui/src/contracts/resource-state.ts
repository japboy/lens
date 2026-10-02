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
  | { stage: "allowed" }
  | { stage: "required" }
  | { stage: "failed"; message: string };
export type SnapshotStatus =
  | { stage: "loading" }
  | { stage: "ready" }
  | { stage: "failed"; message: string };
