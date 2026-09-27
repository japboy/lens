import { expect, it, vi } from "vitest";
import { SnapshotResources } from "./snapshot-resources";
import type { AppSnapshot } from "../types";
import type { LensOutputResource, LensSourceResource, WebviewPort } from "./webview-port";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
const snapshot = (
  revision: number,
  source_ref: string | null,
  output_ref: string | null,
  operation_id = "op",
) =>
  ({
    revision,
    source_ref,
    output_ref,
    lens: { operation_id, stage: "transforming", output_blocks: [] },
  }) as unknown as AppSnapshot;

it("admits lightweight progress immediately while coalescing stale source requests", async () => {
  const old = deferred<LensSourceResource>();
  const latest = deferred<LensSourceResource>();
  const port = {
    getLensSource: vi
      .fn<WebviewPort["getLensSource"]>()
      .mockReturnValueOnce(old.promise)
      .mockReturnValueOnce(latest.promise),
  };
  const resources = new SnapshotResources(port as unknown as WebviewPort, vi.fn<() => void>());
  resources.setSourceDemand(true);
  resources.synchronize(snapshot(1, "old", null));
  resources.synchronize(snapshot(2, "skipped", null));
  resources.synchronize(snapshot(3, "latest", null));
  expect(port.getLensSource).toHaveBeenCalledTimes(1);
  expect(resources.project(snapshot(3, "latest", null)).revision).toBe(3);
  old.resolve({ source_ref: "old" });
  await vi.waitFor(() => expect(port.getLensSource.mock.calls).toEqual([["old"], ["latest"]]));
  latest.resolve({ source_ref: "latest", input: { marker: "current" } as never });
  await vi.waitFor(() => expect(resources.sourceState.stage).toBe("ready"));
  const body = resources.project(snapshot(3, "latest", null)).lens.input;
  resources.synchronize(snapshot(4, "latest", null));
  expect(resources.project(snapshot(4, "latest", null)).lens.input).toBe(body);
  expect(port.getLensSource).toHaveBeenCalledTimes(2);
});

it("rejects source completion after disconnect even when reconnect reuses the same ref", async () => {
  const old = deferred<LensSourceResource>();
  const fresh = deferred<LensSourceResource>();
  const port = {
    getLensSource: vi
      .fn<WebviewPort["getLensSource"]>()
      .mockReturnValueOnce(old.promise)
      .mockReturnValueOnce(fresh.promise),
  };
  const resources = new SnapshotResources(port as unknown as WebviewPort, vi.fn<() => void>());
  resources.setSourceDemand(true);
  resources.synchronize(snapshot(1, "same", null));
  resources.clear();
  resources.setSourceDemand(true);
  resources.synchronize(snapshot(1, "same", null));
  old.resolve({ source_ref: "same", input: { marker: "stale" } as never });
  await vi.waitFor(() => expect(port.getLensSource).toHaveBeenCalledTimes(2));
  expect(resources.project(snapshot(1, "same", null)).lens.input).toBeUndefined();
  fresh.resolve({ source_ref: "same", input: { marker: "fresh" } as never });
  await vi.waitFor(() =>
    expect(resources.project(snapshot(1, "same", null)).lens.input).toEqual({ marker: "fresh" }),
  );
});

it("reuses image bytes through text updates and clears old output at operation replacement", async () => {
  const image = {
    type: "image",
    id: "image",
    mime_type: "image/png",
    data: "",
    image_ref: "image-ref",
  } as const;
  const port = {
    getLensOutput: vi.fn<WebviewPort["getLensOutput"]>(
      async (ref: string): Promise<LensOutputResource> => ({
        output_ref: ref,
        output_blocks: [image],
      }),
    ),
    getLensImage: vi.fn<WebviewPort["getLensImage"]>(async () => ({
      image_ref: "image-ref",
      data: "large-image",
    })),
  };
  const resources = new SnapshotResources(port as unknown as WebviewPort, vi.fn<() => void>());
  resources.setSourceDemand(true);
  resources.synchronize(snapshot(1, null, "one"));
  await vi.waitFor(() => expect(resources.outputState.stage).toBe("ready"));
  const first = resources.project(snapshot(1, null, "one")).lens.output_blocks;
  resources.synchronize(snapshot(2, null, "one"));
  expect(resources.project(snapshot(2, null, "one")).lens.output_blocks).toBe(first);
  resources.synchronize(snapshot(3, null, "two"));
  await vi.waitFor(() => expect(resources.outputState.stage).toBe("ready"));
  expect(port.getLensImage).toHaveBeenCalledTimes(1);
  resources.synchronize(snapshot(4, null, null, "new-op"));
  expect(resources.project(snapshot(4, null, null, "new-op")).lens.output_blocks).toEqual([]);
});

it("reports mismatched resource identities without admitting the body", async () => {
  const resources = new SnapshotResources(
    {
      getLensSource: async () => ({ source_ref: "wrong", input: {} }),
    } as unknown as WebviewPort,
    vi.fn<() => void>(),
  );
  resources.setSourceDemand(true);
  resources.synchronize(snapshot(1, "expected", null));
  await vi.waitFor(() => expect(resources.sourceState.stage).toBe("failed"));
  expect(resources.project(snapshot(1, "expected", null)).lens.input).toBeUndefined();
});

it("does not display provisional output under a replacement run", async () => {
  const pending = deferred<LensOutputResource>();
  const resources = new SnapshotResources(
    {
      getLensOutput: vi
        .fn<WebviewPort["getLensOutput"]>()
        .mockResolvedValueOnce({
          output_ref: "one",
          output_blocks: [{ type: "markdown", text: "old" }],
        })
        .mockReturnValueOnce(pending.promise),
    } as unknown as WebviewPort,
    vi.fn<() => void>(),
  );
  const first = snapshot(1, null, "one");
  first.lens.agent = { run_id: "old-run" } as never;
  resources.synchronize(first);
  await vi.waitFor(() => expect(resources.outputState.stage).toBe("ready"));
  const next = snapshot(2, null, "two");
  next.lens.agent = { run_id: "new-run" } as never;
  resources.synchronize(next);
  expect(resources.project(next).lens.output_blocks).toEqual([]);
  pending.resolve({ output_ref: "two", output_blocks: [] });
});

it("retries a failed current resource only after explicit local retry", async () => {
  const load = vi
    .fn<WebviewPort["getLensSource"]>()
    .mockRejectedValueOnce(new Error("transient"))
    .mockResolvedValueOnce({ source_ref: "same" });
  const resources = new SnapshotResources(
    { getLensSource: load } as unknown as WebviewPort,
    vi.fn<() => void>(),
  );
  resources.setSourceDemand(true);
  resources.synchronize(snapshot(1, "same", null));
  await vi.waitFor(() => expect(resources.sourceState.stage).toBe("failed"));
  resources.synchronize(snapshot(2, "same", null));
  expect(load).toHaveBeenCalledTimes(1);
  resources.retry("source");
  await vi.waitFor(() => expect(resources.sourceState.stage).toBe("ready"));
  expect(load).toHaveBeenCalledTimes(2);
});

it("clears resource loading state when authority removes its reference", async () => {
  const pending = deferred<LensSourceResource>();
  const resources = new SnapshotResources(
    { getLensSource: () => pending.promise } as unknown as WebviewPort,
    vi.fn<() => void>(),
  );
  resources.setSourceDemand(true);
  resources.synchronize(snapshot(1, "old", null));
  expect(resources.sourceState.stage).toBe("loading");
  resources.synchronize(snapshot(2, null, null));
  expect(resources.sourceState.stage).toBe("idle");
  pending.resolve({ source_ref: "old" });
  await Promise.resolve();
  expect(resources.project(snapshot(2, null, null)).lens.input).toBeUndefined();
});

it("fetches only the latest source when its panel requests content", async () => {
  const load = vi.fn<WebviewPort["getLensSource"]>(async (source_ref) => ({ source_ref }));
  const resources = new SnapshotResources(
    { getLensSource: load } as unknown as WebviewPort,
    vi.fn<() => void>(),
  );
  resources.synchronize(snapshot(1, "hidden-old", null));
  resources.synchronize(snapshot(2, "visible-current", null));
  expect(load).not.toHaveBeenCalled();
  resources.setSourceDemand(true);
  await vi.waitFor(() => expect(resources.sourceState.stage).toBe("ready"));
  expect(load.mock.calls).toEqual([["visible-current"]]);
  resources.setSourceDemand(false);
  resources.synchronize(snapshot(3, "hidden-next", null));
  expect(load).toHaveBeenCalledTimes(1);
  expect(resources.project(snapshot(3, "hidden-next", null)).lens.input).toBeUndefined();
  resources.setSourceDemand(true);
  await vi.waitFor(() => expect(load.mock.calls).toEqual([["visible-current"], ["hidden-next"]]));
});

it("reuses a slow immutable image when a newer text reference arrives before it resolves", async () => {
  const image = deferred<{ image_ref: string; data: string }>();
  const getLensImage = vi.fn<WebviewPort["getLensImage"]>(() => image.promise);
  const getLensOutput = vi.fn<WebviewPort["getLensOutput"]>(async (output_ref) => ({
    output_ref,
    output_blocks: [{ type: "image", mime_type: "image/png", data: "", image_ref: "shared-image" }],
  }));
  const resources = new SnapshotResources(
    { getLensOutput, getLensImage } as unknown as WebviewPort,
    vi.fn<() => void>(),
  );
  resources.synchronize(snapshot(1, null, "old-text"));
  await vi.waitFor(() => expect(getLensImage).toHaveBeenCalledTimes(1));
  resources.synchronize(snapshot(2, null, "latest-text"));
  image.resolve({ image_ref: "shared-image", data: "large-bytes" });
  await vi.waitFor(() => expect(resources.outputState.stage).toBe("ready"));
  expect(getLensOutput.mock.calls).toEqual([["old-text"], ["latest-text"]]);
  expect(getLensImage).toHaveBeenCalledTimes(1);
  expect(resources.project(snapshot(2, null, "latest-text")).lens.output_blocks).toEqual([
    { type: "image", mime_type: "image/png", image_ref: "shared-image", data: "large-bytes" },
  ]);
});
