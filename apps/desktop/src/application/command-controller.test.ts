import { expect, it, vi } from "vitest";
import type { ReactiveControllerHost } from "lit";
import { CommandController } from "./command-controller";

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}
const host = () =>
  ({
    addController: vi.fn<ReactiveControllerHost["addController"]>(),
    requestUpdate: vi.fn<ReactiveControllerHost["requestUpdate"]>(),
  }) as unknown as ReactiveControllerHost;

it("retains independent Settings pending and failures while rejecting conflicting operations", async () => {
  const commands = new CommandController(host());
  const connection = deferred();
  const prompt = deferred();
  const first = commands.run({ scope: "settings", type: "select-agent" }, () => connection.promise);
  const second = commands.run(
    { scope: "settings", type: "update-prompt-presets" },
    () => prompt.promise,
  );
  const conflicting = vi.fn<() => Promise<void>>(async () => {});
  await commands.run({ scope: "settings", type: "save-agent-defaults" }, conflicting);
  expect(conflicting).not.toHaveBeenCalled();
  expect(commands.states.filter((state) => state.stage === "pending")).toHaveLength(2);
  connection.reject(new Error("Connection failed"));
  await first;
  prompt.resolve();
  await second;
  expect(commands.states).toContainEqual({
    stage: "failed",
    command: { scope: "settings", type: "select-agent" },
    message: "Error: Connection failed",
  });
});

it("keeps Close pending through unrelated link completion and suppresses duplicate Stop", async () => {
  const commands = new CommandController(host());
  const stop = deferred();
  const first = commands.run({ scope: "overlay", type: "close" }, () => stop.promise);
  await commands.run({ scope: "overlay", type: "open-external-url" }, async () => {});
  const duplicate = vi.fn<() => Promise<void>>(async () => {});
  await commands.run({ scope: "overlay", type: "close" }, duplicate);
  expect(duplicate).not.toHaveBeenCalled();
  expect(commands.state).toEqual({
    stage: "pending",
    command: { scope: "overlay", type: "close" },
  });
  commands.hostDisconnected();
  stop.reject(new Error("stale"));
  await first;
  expect(commands.state).toEqual({ stage: "idle" });
});
