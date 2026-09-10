import { describe, expect, it, vi } from "vitest";
import type { ReactiveControllerHost } from "lit";
import type { LensState } from "../types";
import { HtmlOutputController, MAX_HTML_OUTPUT_BYTES } from "./html-output-controller";

function lens(representationId = "rep", resourceId = "html", bytes = 3): LensState {
  return {
    operation_id: "op",
    stage: "completed",
    prompt_execution_revision: 1,
    output_blocks: [],
    representation: {
      representation_id: representationId,
      context_id: "context",
      context_revision: 1,
      projection: { revision: 1, digest: "digest" },
      run_id: "run",
      prompt_execution_revision: 1,
      output_blocks: [
        {
          type: "html",
          resource_id: resourceId,
          mime_type: "text/html",
          uri: "lens:html",
          byte_length: bytes,
        },
      ],
    },
  };
}

function setup(getHtmlOutput = vi.fn<() => Promise<string>>(async () => "abc")) {
  const host: ReactiveControllerHost = {
    addController: vi.fn<ReactiveControllerHost["addController"]>(),
    removeController: vi.fn<ReactiveControllerHost["removeController"]>(),
    requestUpdate: vi.fn<ReactiveControllerHost["requestUpdate"]>(),
    updateComplete: Promise.resolve(true),
  };
  const controller = new HtmlOutputController(host, { getHtmlOutput });
  return { controller, getHtmlOutput };
}

describe("HtmlOutputController", () => {
  it("loads published content once and retains it during a live candidate", async () => {
    const { controller, getHtmlOutput } = setup();
    const current = lens();
    controller.synchronize(current);
    expect(controller.content).toEqual({ resourceId: "html", status: "loading" });
    await vi.waitFor(() =>
      expect(controller.content).toEqual({ resourceId: "html", status: "ready", content: "abc" }),
    );
    controller.synchronize({
      ...current,
      stage: "transforming",
      prompt_execution_revision: 1,
      output_blocks: [{ type: "markdown", text: "candidate" }],
    });
    expect(getHtmlOutput).toHaveBeenCalledExactlyOnceWith("op", "rep", "html");
    expect(controller.content?.status).toBe("ready");
    controller.synchronize({ ...current, representation: undefined });
    expect(controller.content).toBeUndefined();
  });

  it("ignores private descriptors until a representation is published", () => {
    const { controller, getHtmlOutput } = setup();
    const current = lens();
    controller.synchronize({
      ...current,
      representation: undefined,
      prompt_execution_revision: 1,
      output_blocks: current.representation!.output_blocks,
    });
    expect(getHtmlOutput).not.toHaveBeenCalled();
    expect(controller.content).toBeUndefined();
  });

  it("ignores stale loads after replacement and disconnect", async () => {
    const resolvers: Array<(value: string) => void> = [];
    const { controller } = setup(
      vi.fn(() => new Promise<string>((resolve) => resolvers.push(resolve))),
    );
    controller.synchronize(lens("old", "old"));
    controller.synchronize(lens("new", "new"));
    resolvers[0]!("abc");
    await Promise.resolve();
    expect(controller.content).toEqual({ resourceId: "new", status: "loading" });
    controller.hostDisconnected();
    resolvers[1]!("abc");
    await Promise.resolve();
    expect(controller.content).toBeUndefined();
  });

  it("reports fetch failures locally and validates UTF-8 size", async () => {
    const { controller } = setup(vi.fn<() => Promise<string>>(async () => "\u3042"));
    controller.synchronize(lens());
    await vi.waitFor(() => expect(controller.content?.status).toBe("ready"));
    controller.synchronize(lens("bad", "bad", 1));
    await vi.waitFor(() => expect(controller.content?.status).toBe("failed"));
    const failed = setup(
      vi.fn(async () => {
        throw new Error("private backend detail");
      }),
    );
    failed.controller.synchronize(lens());
    await vi.waitFor(() =>
      expect(failed.controller.content).toEqual({
        resourceId: "html",
        status: "failed",
        message: "HTML content could not be loaded.",
      }),
    );
  });

  it("rejects oversized descriptors and multiple resources before IPC", () => {
    const { controller, getHtmlOutput } = setup();
    controller.synchronize(lens("large", "large", MAX_HTML_OUTPUT_BYTES + 1));
    expect(controller.content?.status).toBe("failed");
    const multiple = lens();
    multiple.representation!.output_blocks.push({ ...multiple.representation!.output_blocks[0]! });
    controller.synchronize(multiple);
    expect(controller.content?.status).toBe("failed");
    expect(getHtmlOutput).not.toHaveBeenCalled();
  });
});
