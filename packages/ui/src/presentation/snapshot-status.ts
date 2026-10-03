import { html, nothing } from "lit";
import type { SnapshotConnectionState } from "../contracts/resource-state";

import type { SnapshotStatus } from "../contracts/resource-state";

export function snapshotStatus(
  snapshot: object | undefined,
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
