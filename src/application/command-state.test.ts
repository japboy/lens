import { describe, expect, it } from "vitest";
import { canStartCommand, type CommandState } from "./command-state";

describe("command concurrency policy", () => {
  it("allows cancellation to interrupt a pending Agent operation", () => {
    const retry: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "retry" },
    };
    const authentication: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "authenticate" },
    };

    expect(canStartCommand(retry, { scope: "overlay", type: "cancel" })).toBe(true);
    expect(canStartCommand(authentication, { scope: "overlay", type: "cancel" })).toBe(true);
  });

  it("rejects duplicate mutations while preserving window and link actions", () => {
    const retry: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "retry" },
    };

    expect(canStartCommand(retry, { scope: "overlay", type: "retry" })).toBe(false);
    expect(canStartCommand(retry, { scope: "overlay", type: "close" })).toBe(true);
    expect(canStartCommand(retry, { scope: "overlay", type: "open-external-url" })).toBe(true);
  });

  it("lets privacy lifecycle controls supersede other work but suppresses duplicates", () => {
    const retry: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "retry" },
    };
    const pause: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "pause" },
    };
    const close: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "close" },
    };

    expect(canStartCommand(retry, { scope: "overlay", type: "pause" })).toBe(true);
    expect(canStartCommand(retry, { scope: "overlay", type: "close" })).toBe(true);
    expect(canStartCommand(pause, { scope: "overlay", type: "pause" })).toBe(false);
    expect(canStartCommand(close, { scope: "overlay", type: "close" })).toBe(false);
  });
});
