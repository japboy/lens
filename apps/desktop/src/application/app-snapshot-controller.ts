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

  private generation = 0;
  private revision = -1;
  private unlisten: Unlisten | undefined;

  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly port: WebviewPort,
  ) {
    host.addController(this);
  }

  hostConnected(): void {
    const generation = ++this.generation;
    void this.load(generation);
  }

  hostDisconnected(): void {
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
        this.applySnapshot(snapshot);
      });
      if (generation !== this.generation) {
        unlisten();
        return;
      }
      this.unlisten = unlisten;

      this.setConnection({ stage: "loading" });
      const snapshot = await this.port.getAppSnapshot();
      if (generation !== this.generation) return;
      this.applySnapshot(snapshot);
      this.setConnection({ stage: "ready" });
    } catch (error) {
      if (generation !== this.generation) return;
      this.setConnection({ stage: "failed", message: String(error) });
    }
  }

  private applySnapshot(snapshot: AppSnapshot): void {
    if (!shouldApplySnapshot(this.revision, snapshot.revision)) return;
    this.revision = snapshot.revision;
    this.snapshot = snapshot;
    this.setConnection({ stage: "ready" });
  }

  private setConnection(connection: SnapshotConnectionState): void {
    this.connection = connection;
    this.host.requestUpdate();
  }
}
