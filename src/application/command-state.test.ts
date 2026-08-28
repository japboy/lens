import { describe, expect, it } from "vitest";
import { canStartCommand, type CommandState } from "./command-state";

describe("command concurrency policy", () => {
  it("allows cancellation to interrupt a pending Agent operation", () => {
    const transform: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "transform" },
    };
    const authentication: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "authenticate" },
    };

    expect(canStartCommand(transform, { scope: "overlay", type: "cancel" })).toBe(true);
    expect(canStartCommand(authentication, { scope: "overlay", type: "cancel" })).toBe(true);
  });

  it("rejects duplicate mutations while preserving window and link actions", () => {
    const transform: CommandState = {
      stage: "pending",
      command: { scope: "overlay", type: "transform" },
    };

    expect(canStartCommand(transform, { scope: "overlay", type: "transform" })).toBe(false);
    expect(canStartCommand(transform, { scope: "overlay", type: "close" })).toBe(true);
    expect(canStartCommand(transform, { scope: "overlay", type: "open-external-url" })).toBe(true);
  });
});
