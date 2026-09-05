import { describe, expect, it, vi } from "vitest";
import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { AppSnapshot } from "../types";
import { AppSnapshotController } from "./app-snapshot-controller";
import type { WebviewPort } from "./webview-port";

const snapshot = (revision: number): AppSnapshot => ({
  revision,
  config: {
    agent: "codex",
    working_directory: "/tmp",
    agent_prompt_template: {
      schema_version: 1,
      common: "Transform the selected content.\n\n{turn_instruction}",
      full_projection: "Use the initial projection.",
      source_checkpoint: "Replace revision {base_revision} with {target_revision}.",
      current_projection_retry: "Retry revision {applied_revision}.",
    },
  },
  agent_selection: { stage: "unselected", auth_methods: [] },
  agent_runtime: { stage: "not_installed", downloaded_bytes: 0 },
  lens: { stage: "idle", output_blocks: [] },
});

class TestHost implements ReactiveControllerHost {
  readonly controllers: ReactiveController[] = [];
  readonly requestUpdate = vi.fn<() => void>();
  readonly updateComplete = Promise.resolve(true);

  addController(controller: ReactiveController): void {
    this.controllers.push(controller);
  }

  removeController(controller: ReactiveController): void {
    const index = this.controllers.indexOf(controller);
    if (index >= 0) this.controllers.splice(index, 1);
  }
}

describe("AppSnapshotController", () => {
  it("subscribes before fetching and rejects a late stale fetch response", async () => {
    const calls: string[] = [];
    let listener: ((value: AppSnapshot) => void) | undefined;
    let resolveFetch: ((value: AppSnapshot) => void) | undefined;
    const port = {
      async subscribeToAppSnapshot(next: (value: AppSnapshot) => void) {
        calls.push("subscribe");
        listener = next;
        return () => undefined;
      },
      getAppSnapshot() {
        calls.push("fetch");
        return new Promise<AppSnapshot>((resolve) => {
          resolveFetch = resolve;
        });
      },
    } as WebviewPort;
    const host = new TestHost();
    const controller = new AppSnapshotController(host, port);

    controller.hostConnected();
    await vi.waitFor(() => expect(calls).toEqual(["subscribe", "fetch"]));
    listener?.(snapshot(2));
    resolveFetch?.(snapshot(1));
    await vi.waitFor(() => expect(controller.connection.stage).toBe("ready"));

    expect(controller.snapshot?.revision).toBe(2);
    expect(host.requestUpdate).toHaveBeenCalled();
    controller.hostDisconnected();
  });
});
