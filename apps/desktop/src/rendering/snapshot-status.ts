import { html, nothing } from "lit";
import type { AppSnapshot } from "../types";
import type { SnapshotConnectionState } from "../application/app-snapshot-controller";

export type SnapshotStatus =
  | { stage: "loading" }
  | { stage: "ready" }
  | { stage: "failed"; message: string };

export function snapshotStatus(
  snapshot: AppSnapshot | undefined,
  connection: SnapshotConnectionState,
): SnapshotStatus {
  if (snapshot) return { stage: "ready" };
  return connection.stage === "failed"
    ? { stage: "failed", message: connection.message }
    : { stage: "loading" };
}

export function renderSnapshotFailure(status: SnapshotStatus) {
  return status.stage === "failed" ? html`<p role="alert">${status.message}</p>` : nothing;
}
