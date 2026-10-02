import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { AppSnapshot } from "../types";
import { shouldApplySnapshot } from "./snapshot-admission";
import type { Unlisten, WebviewPort } from "./webview-port";
import { SnapshotResources } from "./snapshot-resources";

import type { SnapshotConnectionState } from "ui/contracts/resource-state";

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
  private received: AppSnapshot | undefined;
  readonly resources: SnapshotResources;
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
    this.resources = new SnapshotResources(port, () => {
      if (!this.connected || !this.active || !this.received) return;
      this.snapshot = this.resources.project(this.received);
      this.host.requestUpdate();
    });
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
    this.resources.clear();
    this.received = undefined;
    this.revision = -1;
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
    this.received = snapshot;
    this.resources.synchronize(snapshot);
    this.snapshot = this.resources.project(snapshot);
    this.setConnection({ stage: "ready" });
  }

  private setConnection(connection: SnapshotConnectionState): void {
    this.connection = connection;
    this.host.requestUpdate();
  }
}
