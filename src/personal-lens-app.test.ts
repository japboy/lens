// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { AppSnapshot } from "./types";

const snapshot: AppSnapshot = {
  revision: 1,
  config: {
    agent: "codex",
    response_prompt: "Transform the selected content.",
    working_directory: "/tmp",
  },
  agent_selection: {
    stage: "selected",
    candidate: "codex",
    auth_methods: [],
  },
  agent_runtime: {
    stage: "ready",
    agent: "codex",
    downloaded_bytes: 0,
  },
  lens: {
    operation_id: "operation-1",
    stage: "completed",
    output_blocks: [
      { type: "markdown", message_id: "message-1", text: "Before image" },
      {
        type: "image",
        message_id: "message-1",
        mime_type: "image/png",
        data: "iVBORw0KGgo=",
      },
      { type: "markdown", message_id: "message-1", text: "After image" },
    ],
  },
};

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn<(command: string) => Promise<unknown>>(async (command: string) => {
    if (command === "get_app_snapshot") return snapshot;
    if (command === "accessibility_permission") return true;
    return undefined;
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn<() => Promise<() => void>>(async () => () => undefined),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ close: vi.fn<() => void>() }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn<() => Promise<boolean>>(),
  open: vi.fn<() => Promise<null>>(),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn<(url: string) => Promise<void>>(),
}));

beforeAll(() => {
  window.history.replaceState({}, "", "/?view=overlay");
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  globalThis.requestAnimationFrame ??= (callback: FrameRequestCallback) =>
    window.setTimeout(() => callback(performance.now()), 0);
  globalThis.cancelAnimationFrame ??= (handle: number) => window.clearTimeout(handle);
  HTMLElement.prototype.scrollTo ??= () => undefined;
});

afterEach(() => {
  document.body.replaceChildren();
});

describe("PersonalLens rich Agent output", () => {
  it("renders ACP image data inline and preserves surrounding block order", async () => {
    await import("./personal-lens-app");
    const element = document.createElement("personal-lens-app") as HTMLElement & {
      updateComplete: Promise<boolean>;
    };
    document.body.append(element);
    await element.updateComplete;
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector(".lens-output img")).not.toBeNull();
    });

    const output = element.shadowRoot?.querySelector(".lens-output");
    const blocks = Array.from(output?.children ?? []);
    const image = output?.querySelector("img");

    expect(blocks.map((block) => block.tagName.toLowerCase())).toEqual([
      "personal-lens-markdown",
      "figure",
      "personal-lens-markdown",
    ]);
    expect(image?.getAttribute("src")).toBe("data:image/png;base64,iVBORw0KGgo=");
    expect(image?.getAttribute("alt")).toBe("Visual output from the agent");
  });
});
