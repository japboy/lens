import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { AppSnapshot } from "../types";
import { shouldApplySnapshot } from "../view-model";
import type { Unlisten, WebviewPort } from "./webview-port";

export type SnapshotConnectionState =
  | { stage: "subscribing" }
  | { stage: "loading" }
  | { stage: "ready" }
  | { stage: "failed"; message: string };

export function snapshotConnectionMessage(connection: SnapshotConnectionState): string {
  switch (connection.stage) {
    case "subscribing":
      return "Subscribing to application state…";
    case "loading":
      return "Loading application state…";
    case "ready":
      return "";
    case "failed":
      return connection.message;
  }
}

export class AppSnapshotController implements ReactiveController {
  snapshot: AppSnapshot | undefined;
  connection: SnapshotConnectionState = { stage: "subscribing" };

  private connected = false;
  private generation = 0;
  private revision = -1;
  private unlisten: Unlisten | undefined;

  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly port: WebviewPort,
    private active = true,
  ) {
    host.addController(this);
  }

  hostConnected(): void {
    this.connected = true;
    if (this.active) void this.load(++this.generation);
  }

  setActive(active: boolean): void {
    if (active === this.active) return;
    this.active = active;
    if (!active) this.stop();
    else if (this.connected) void this.load(++this.generation);
  }

  hostDisconnected(): void {
    this.connected = false;
    this.stop();
  }

  private stop(): void {
    this.generation += 1;
    this.unlisten?.();
    this.unlisten = undefined;
  }

  message(): string {
    return snapshotConnectionMessage(this.connection);
  }

  private async load(generation: number): Promise<void> {
    try {
      this.setConnection({ stage: "subscribing" });
      const unlisten = await this.port.subscribeToAppSnapshot((snapshot) => {
        if (generation === this.generation) this.applySnapshot(snapshot);
      });
      if (generation !== this.generation) {
        unlisten();
        return;
      }
      this.unlisten = unlisten;

      if (this.connection.stage === "subscribing") this.setConnection({ stage: "loading" });
      const snapshot = await this.port.getAppSnapshot();
      if (generation !== this.generation) return;
      this.applySnapshot(snapshot);
    } catch (error) {
      if (generation !== this.generation) return;
      this.setConnection({ stage: "failed", message: String(error) });
    }
  }

  private applySnapshot(snapshot: AppSnapshot): void {
    const selection = snapshot.lens.selection;
    const uuid = (value: unknown): value is string =>
      typeof value === "string" &&
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value) &&
      value !== "00000000-0000-0000-0000-000000000000";
    if (
      selection &&
      (!uuid(selection.selection_id) ||
        selection.selection_id !== snapshot.lens.operation_id ||
        !Array.isArray(selection.items) ||
        selection.items.some((item) => {
          const window = item?.window;
          return (
            !window ||
            typeof window !== "object" ||
            window.operation_id !== selection.selection_id ||
            !uuid(window.receipt) ||
            !Number.isInteger(window.selection_ordinal) ||
            window.selection_ordinal <= 0 ||
            window.selection_ordinal > 0xffffffff ||
            typeof window.application_id !== "string" ||
            "window_id" in window ||
            "pid" in window ||
            "bundle_id" in window
          );
        }))
    ) {
      this.setConnection({ stage: "failed", message: "Unsupported LensTargetSelection identity" });
      return;
    }
    const versions = [
      ["LensTargetSet", snapshot.lens.target_set, 3],
      ["LensContext", snapshot.lens.context, 9],
      ["LensInput", snapshot.lens.input, 8],
      ...(snapshot.lens.context?.sources ?? []).map(
        (source) => ["LensDocument", source.document, 6] as const,
      ),
    ] as const;
    for (const [name, value, expected] of versions) {
      if (value !== undefined && value !== null && value.schema_version !== expected) {
        this.setConnection({ stage: "failed", message: `Unsupported ${name} schema version` });
        return;
      }
    }
    for (const resource of [snapshot.lens.context, snapshot.lens.input]) {
      if (
        resource?.media.some(
          (item) =>
            item.scope !== "accessibility_element_region" && item.scope !== "window_fallback",
        )
      ) {
        this.setConnection({ stage: "failed", message: "Unsupported Lens media scope" });
        return;
      }
    }
    if (!shouldApplySnapshot(this.revision, snapshot.revision)) {
      if (this.snapshot !== undefined && this.connection.stage !== "failed") {
        this.setConnection({ stage: "ready" });
      }
      return;
    }
    this.revision = snapshot.revision;
    this.snapshot = snapshot;
    this.setConnection({ stage: "ready" });
  }

  private setConnection(connection: SnapshotConnectionState): void {
    this.connection = connection;
    this.host.requestUpdate();
  }
}
