import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
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
  it("rejects old selection-only identity without consuming its higher revision", async () => {
    let listener: ((value: AppSnapshot) => void) | undefined;
    const port = {
      async subscribeToAppSnapshot(value: (value: AppSnapshot) => void) {
        listener = value;
        return () => undefined;
      },
      async getAppSnapshot() {
        return snapshot(1);
      },
    } as WebviewPort;
    const controller = new AppSnapshotController(new TestHost(), port);
    controller.hostConnected();
    await vi.waitFor(() => expect(controller.connection.stage).toBe("ready"));
    const operation = "33333333-3333-4333-8333-333333333333";
    const valid = snapshot(2);
    valid.lens.operation_id = operation;
    valid.lens.stage = "selecting";
    valid.lens.selection = {
      selection_id: operation,
      stage: "reviewing",
      maximum_targets: 4,
      anchor: { x: 0, y: 0, width: 10, height: 10 },
      items: [
        {
          id: "target",
          window: {
            operation_id: operation,
            receipt: "11111111-1111-4111-8111-111111111111",
            selection_ordinal: 1,
            application_id: "example.fixture",
            application_name: "Fixture",
            title: "Fixture",
            frame: { x: 0, y: 0, width: 10, height: 10 },
          },
        },
      ],
    };
    const obsolete = structuredClone(valid);
    obsolete.revision = 99;
    obsolete.lens.selection!.items[0]!.window = {
      window_id: 42,
      pid: 100,
      bundle_id: "example.fixture",
    } as unknown as (typeof valid.lens.selection.items)[0]["window"];
    expect(() => listener?.(obsolete)).not.toThrow();
    expect(controller.connection.stage).toBe("failed");
    expect(controller.snapshot?.revision).toBe(1);
    listener?.(valid);
    expect(controller.snapshot?.revision).toBe(2);
    expect(controller.connection.stage).toBe("ready");
    controller.hostDisconnected();
  });
  it.each(["context", "input"] as const)(
    "rejects native AX media scope in current %s schema",
    async (resource) => {
      const fixture = JSON.parse(
        readFileSync(
          new URL("../../tests/fixtures/workspace-contracts.json", import.meta.url),
          "utf8",
        ),
      );
      const invalid = snapshot(2);
      invalid.lens[resource] = { ...fixture[resource], media: [{ scope: "ax_element_region" }] };
      const port = {
        async subscribeToAppSnapshot() {
          return () => undefined;
        },
        async getAppSnapshot() {
          return invalid;
        },
      } as unknown as WebviewPort;
      const controller = new AppSnapshotController(new TestHost(), port);
      controller.hostConnected();
      await vi.waitFor(() =>
        expect(controller.connection).toEqual({
          stage: "failed",
          message: "Unsupported Lens media scope",
        }),
      );
      expect(controller.snapshot).toBeUndefined();
      controller.hostDisconnected();
    },
  );
  it.each(["reconnect", "reactivate"])(
    "settles ready after %s fetches the same revision",
    async (transition) => {
      const value = snapshot(1);
      const port = {
        subscribeToAppSnapshot: vi
          .fn<WebviewPort["subscribeToAppSnapshot"]>()
          .mockResolvedValue(() => undefined),
        async getAppSnapshot() {
          return value;
        },
      } as unknown as WebviewPort;
      const controller = new AppSnapshotController(new TestHost(), port);
      controller.hostConnected();
      await vi.waitFor(() => expect(controller.connection.stage).toBe("ready"));
      if (transition === "reconnect") {
        controller.hostDisconnected();
        controller.hostConnected();
      } else {
        controller.setActive(false);
        controller.setActive(true);
      }
      await vi.waitFor(() => expect(controller.connection.stage).toBe("ready"));
      expect(controller.snapshot).toBe(value);
      controller.hostDisconnected();
    },
  );

  it("preserves a synchronously admitted subscription snapshot through equal fetch", async () => {
    const value = snapshot(2);
    const port = {
      async subscribeToAppSnapshot(listener: (value: AppSnapshot) => void) {
        listener(value);
        return () => undefined;
      },
      async getAppSnapshot() {
        return value;
      },
    } as WebviewPort;
    const controller = new AppSnapshotController(new TestHost(), port);
    controller.hostConnected();
    await vi.waitFor(() => expect(controller.connection.stage).toBe("ready"));
    await Promise.resolve();
    expect(controller.connection.stage).toBe("ready");
    expect(controller.snapshot).toBe(value);
    controller.hostDisconnected();
  });

  it.each(["fetch", "event"])("admits Rust null resource shapes from %s", async (delivery) => {
    const fixture = JSON.parse(
      readFileSync(
        new URL("../../tests/fixtures/workspace-contracts.json", import.meta.url),
        "utf8",
      ),
    ) as { snapshot: AppSnapshot; context: NonNullable<AppSnapshot["lens"]["context"]> };
    const idle = { ...fixture.snapshot, revision: 2 };
    expect(idle.lens.target_set).toBeNull();
    expect(idle.lens.context).toBeNull();
    expect(idle.lens.input).toBeNull();
    let listener: ((value: AppSnapshot) => void) | undefined;
    const port = {
      async subscribeToAppSnapshot(next: (value: AppSnapshot) => void) {
        listener = next;
        return () => undefined;
      },
      async getAppSnapshot() {
        return delivery === "fetch" ? idle : snapshot(1);
      },
    } as WebviewPort;
    const controller = new AppSnapshotController(new TestHost(), port);
    controller.hostConnected();
    await vi.waitFor(() => expect(controller.connection.stage).toBe("ready"));
    expect(() => {
      if (delivery === "event") listener?.(idle);
    }).not.toThrow();
    expect(controller.snapshot).toEqual(idle);
    const unavailableDocument: AppSnapshot = {
      ...idle,
      revision: 3,
      lens: {
        ...idle.lens,
        context: {
          ...fixture.context,
          schema_version: 9,
          sources: fixture.context.sources.map((source) => ({ ...source, document: null })),
        },
      },
    };
    expect(() => listener?.(unavailableDocument)).not.toThrow();
    expect(controller.snapshot).toEqual(unavailableDocument);
    expect(controller.connection.stage).toBe("ready");
    controller.hostDisconnected();
  });

  it.each(
    ["fetch", "event"].flatMap((delivery) => [6, 7].map((version) => ({ delivery, version }))),
  )(
    "rejects obsolete LensInput $version from $delivery without advancing revision",
    async ({ delivery, version }) => {
      let listener: ((value: AppSnapshot) => void) | undefined;
      const outdated = snapshot(2);
      outdated.lens.input = {
        schema_version: version,
        context_id: "context",
        context_revision: 1,
        sources: [],
        media: [],
        media_omissions: [],
        quality: "full",
      } as unknown as NonNullable<AppSnapshot["lens"]["input"]>;
      const port = {
        async subscribeToAppSnapshot(next: (value: AppSnapshot) => void) {
          listener = next;
          return () => undefined;
        },
        async getAppSnapshot() {
          return delivery === "fetch" ? outdated : snapshot(1);
        },
      } as WebviewPort;
      const controller = new AppSnapshotController(new TestHost(), port);
      controller.hostConnected();
      await vi.waitFor(() =>
        expect(controller.connection.stage).toBe(delivery === "fetch" ? "failed" : "ready"),
      );
      if (delivery === "event") listener?.(outdated);
      expect(controller.connection).toEqual({
        stage: "failed",
        message: "Unsupported LensInput schema version",
      });
      expect(controller.snapshot?.revision).not.toBe(2);
      if (delivery === "event") listener?.(snapshot(1));
      expect(controller.connection.stage).toBe("failed");
      listener?.(snapshot(2));
      expect(controller.snapshot?.revision).toBe(2);
      expect(controller.connection.stage).toBe("ready");
      controller.hostDisconnected();
    },
  );

  it.each(
    ["fetch", "event"].flatMap((delivery) =>
      [
        { resource: "LensContext", version: 7 },
        { resource: "LensContext", version: 8 },
        { resource: "LensContext", version: 99 },
        { resource: "LensDocument", version: 4 },
        { resource: "LensDocument", version: 5 },
        { resource: "LensDocument", version: 99 },
      ].map((entry) => ({ delivery, ...entry })),
    ),
  )(
    "rejects $resource version $version from $delivery",
    async ({ delivery, resource, version }) => {
      const fixture = JSON.parse(
        readFileSync(
          new URL("../../tests/fixtures/workspace-contracts.json", import.meta.url),
          "utf8",
        ),
      ) as { context: NonNullable<AppSnapshot["lens"]["context"]> };
      const valid = snapshot(3);
      valid.lens.context = {
        ...fixture.context,
        schema_version: 9,
        sources: fixture.context.sources.map((source) => ({
          ...source,
          document: source.document ? { ...source.document, schema_version: 6 } : null,
        })),
      };
      const invalid = structuredClone(valid);
      invalid.revision = 2;
      const context = invalid.lens.context!;
      const versioned: { schema_version: number } =
        resource === "LensContext" ? context : context.sources[0]!.document!;
      versioned.schema_version = version;
      let listener: ((value: AppSnapshot) => void) | undefined;
      const port = {
        async subscribeToAppSnapshot(next: (value: AppSnapshot) => void) {
          listener = next;
          return () => undefined;
        },
        async getAppSnapshot() {
          return delivery === "fetch" ? invalid : snapshot(1);
        },
      } as WebviewPort;
      const controller = new AppSnapshotController(new TestHost(), port);
      controller.hostConnected();
      await vi.waitFor(() =>
        expect(controller.connection.stage).toBe(delivery === "fetch" ? "failed" : "ready"),
      );
      if (delivery === "event") listener?.(invalid);
      expect(controller.connection).toEqual({
        stage: "failed",
        message: `Unsupported ${resource} schema version`,
      });
      expect(controller.snapshot?.revision).not.toBe(2);
      listener?.(valid);
      expect(controller.snapshot).toEqual(valid);
      expect(controller.connection.stage).toBe("ready");
      controller.hostDisconnected();
    },
  );

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
