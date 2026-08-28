// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { AppSnapshot } from "./types";

const operationId = "0198e6de-d046-7bf2-b8b2-d84cfaba7e2d";

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
    operation_id: operationId,
    stage: "completed",
    target_set: {
      schema_version: 1,
      selection_id: operationId,
      targets: [
        {
          id: "macos:com.apple.Safari:417",
          window: {
            window_id: 417,
            title: "Fixture",
            application_name: "Safari",
            bundle_id: "com.apple.Safari",
            pid: 417,
            frame: { x: 0, y: 0, width: 800, height: 600 },
          },
        },
        {
          id: "macos:com.apple.TextEdit:512",
          window: {
            window_id: 512,
            title: "Notes",
            application_name: "TextEdit",
            bundle_id: "com.apple.TextEdit",
            pid: 512,
            frame: { x: 80, y: 80, width: 600, height: 500 },
          },
        },
      ],
    },
    input: {
      schema_version: 3,
      context_id: operationId,
      context_revision: 1,
      sources: [
        {
          source_id: "macos:com.apple.Safari:417:accessibility",
          target_id: "macos:com.apple.Safari:417",
          source_revision: 1,
          source: {
            application: "Safari",
            window_title: "Fixture",
            bundle_id: "com.apple.Safari",
            window_id: 417,
          },
          document: {
            nodes: [
              {
                id: "node-000001",
                kind: "image",
                media_refs: ["media-node-000001"],
                resource_refs: [
                  { uri: "https://example.test/first.png", source_attribute: "AXURL" },
                ],
              },
              {
                id: "node-000002",
                kind: "image",
                media_refs: ["media-node-000002"],
              },
            ],
          },
          quality: "full",
          omissions: [],
        },
      ],
      media: [
        {
          id: "media-node-000001",
          target_id: "macos:com.apple.Safari:417",
          uri: `lens://context/${operationId}/1/media/media-node-000001`,
          scope: "ax_element_region",
          source_node_id: "node-000001",
          source_bounds: { x: 10, y: 20, width: 30, height: 40 },
          captured_bounds: { x: 10, y: 20, width: 30, height: 40 },
          coverage: "full_region",
          coordinate_space: "screen_points",
          mime_type: "image/png",
          pixel_width: 300,
          pixel_height: 400,
          encoded_bytes: 512,
        },
        {
          id: "media-node-000002",
          target_id: "macos:com.apple.Safari:417",
          uri: `lens://context/${operationId}/1/media/media-node-000002`,
          scope: "ax_element_region",
          source_node_id: "node-000002",
          source_bounds: { x: 45, y: 50, width: 100, height: 120 },
          captured_bounds: { x: 50, y: 60, width: 70, height: 80 },
          coverage: "visible_subregion",
          coordinate_space: "screen_points",
          mime_type: "image/png",
          pixel_width: 700,
          pixel_height: 800,
          encoded_bytes: 1024,
        },
      ],
      media_omissions: [],
      quality: "full",
    },
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
const completedLens = snapshot.lens;

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
  window.history.replaceState({}, "", "/?view=overlay&platform=macos");
  window.matchMedia ??= () => ({ matches: false }) as MediaQueryList;
  globalThis.requestAnimationFrame ??= (callback: FrameRequestCallback) =>
    window.setTimeout(() => callback(performance.now()), 0);
  globalThis.cancelAnimationFrame ??= (handle: number) => window.clearTimeout(handle);
  HTMLElement.prototype.scrollTo ??= () => undefined;
});

afterEach(() => {
  document.body.replaceChildren();
  window.history.replaceState({}, "", "/?view=overlay&platform=macos");
  snapshot.revision = 1;
  snapshot.lens = completedLens;
  vi.clearAllMocks();
});

describe("Lens target selection preview", () => {
  it("shows only the vertical preview cards and icon actions, then invokes finite edit commands", async () => {
    snapshot.lens = {
      operation_id: operationId,
      stage: "selecting",
      selection: {
        selection_id: operationId,
        stage: "reviewing",
        maximum_targets: 4,
        anchor: { x: 0, y: 0, width: 800, height: 600 },
        items: [
          {
            id: "macos:com.apple.Safari:417",
            window: completedLens.target_set?.targets[0]?.window ?? {
              window_id: 417,
              title: "Fixture",
              application_name: "Safari",
              bundle_id: "com.apple.Safari",
              pid: 417,
              frame: { x: 0, y: 0, width: 800, height: 600 },
            },
            preview_uri: `lens://selection/${operationId}/window/417`,
          },
          {
            id: "macos:com.apple.TextEdit:512",
            window: completedLens.target_set?.targets[1]?.window ?? {
              window_id: 512,
              title: "Notes",
              application_name: "TextEdit",
              bundle_id: "com.apple.TextEdit",
              pid: 512,
              frame: { x: 80, y: 80, width: 600, height: 500 },
            },
            preview_uri: `lens://selection/${operationId}/window/512`,
          },
        ],
      },
      output_blocks: [],
    };
    window.history.replaceState({}, "", "/?view=target-selection&platform=macos");
    await import("./lens-app");
    const { invoke } = await import("@tauri-apps/api/core");
    const element = document.createElement("lens-app") as HTMLElement & {
      updateComplete: Promise<boolean>;
    };
    document.body.append(element);
    await element.updateComplete;
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelectorAll(".target-selection-card")).toHaveLength(2);
    });

    const images = Array.from(
      element.shadowRoot?.querySelectorAll<HTMLImageElement>(".target-selection-image > img") ?? [],
    );
    expect(images.map((image) => image.getAttribute("src"))).toEqual([
      `lens://selection/${operationId}/window/417`,
      `lens://selection/${operationId}/window/512`,
    ]);
    expect(element.shadowRoot?.querySelector(".overlay-header")).toBeNull();
    expect(element.shadowRoot?.querySelector("[data-tauri-drag-region]")).toBeNull();
    expect(element.shadowRoot?.querySelector('[aria-label="Close Lens"]')).toBeNull();
    expect(element.shadowRoot?.querySelector(".target-selection-count")?.textContent).toContain(
      "2 / 4",
    );

    element.shadowRoot
      ?.querySelector<HTMLButtonElement>('[aria-label="Add another window"]')
      ?.click();
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("add_lens_target", { operationId });
    });
    const removeButton = element.shadowRoot?.querySelector<HTMLButtonElement>(
      '[aria-label^="Remove Safari"]',
    );
    await vi.waitFor(() => expect(removeButton?.disabled).toBe(false));
    removeButton?.click();
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("remove_lens_target", {
        operationId,
        targetId: "macos:com.apple.Safari:417",
      });
    });
    const confirmButton = element.shadowRoot?.querySelector<HTMLButtonElement>(
      '[aria-label="Use selected windows"]',
    );
    await vi.waitFor(() => expect(confirmButton?.disabled).toBe(false));
    confirmButton?.click();
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("confirm_lens_targets", { operationId });
    });
  });
});

describe("Lens rich Agent output", () => {
  it("declares the titlebar as the Lens window drag region", async () => {
    await import("./lens-app");
    const element = document.createElement("lens-app") as HTMLElement & {
      updateComplete: Promise<boolean>;
    };
    document.body.append(element);
    await element.updateComplete;
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector(".overlay-title")?.textContent).toContain(
        "2 Windows",
      );
    });

    expect(
      element.shadowRoot?.querySelector(".overlay-header")?.getAttribute("data-tauri-drag-region"),
    ).toBe("deep");
    expect(
      element.shadowRoot?.querySelector(".close-button")?.getAttribute("data-tauri-drag-region"),
    ).toBe("false");
    expect(element.shadowRoot?.querySelector(".overlay-title")?.getAttribute("title")).toContain(
      "TextEdit — Notes",
    );
  });

  it("renders ACP image data inline and preserves surrounding block order", async () => {
    await import("./lens-app");
    const element = document.createElement("lens-app") as HTMLElement & {
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
      "lens-markdown",
      "figure",
      "lens-markdown",
    ]);
    expect(image?.getAttribute("src")).toBe("data:image/png;base64,iVBORw0KGgo=");
    expect(image?.getAttribute("alt")).toBe("Visual output from the agent");
  });

  it("previews only ordered Agent input images without adding payloads to Source JSON", async () => {
    await import("./lens-app");
    const element = document.createElement("lens-app") as HTMLElement & {
      updateComplete: Promise<boolean>;
    };
    document.body.append(element);
    await element.updateComplete;
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector("#source-tab")).not.toBeNull();
    });
    element.shadowRoot?.querySelector<HTMLButtonElement>("#source-tab")?.click();
    await element.updateComplete;

    const preview = element.shadowRoot?.querySelector<HTMLImageElement>(
      ".input-media-preview figure > img",
    );
    const thumbnails = Array.from(
      element.shadowRoot?.querySelectorAll<HTMLButtonElement>(".input-media-thumbnail") ?? [],
    );
    const source = element.shadowRoot?.querySelector(".source-content code")?.textContent;
    expect(preview?.getAttribute("src")).toBe(snapshot.lens.input?.media[0]?.uri);
    expect(preview?.getAttribute("src")).not.toContain("data:");
    expect(thumbnails).toHaveLength(2);
    expect(thumbnails[0]?.getAttribute("aria-current")).toBe("true");
    expect(thumbnails[1]?.querySelector("img")?.getAttribute("src")).toBe(
      snapshot.lens.input?.media[1]?.uri,
    );
    expect(source).toBe(JSON.stringify(snapshot.lens.input, undefined, 2));
    expect(source).not.toContain("iVBORw0KGgo=");

    thumbnails[1]?.click();
    await element.updateComplete;
    expect(
      element.shadowRoot
        ?.querySelector<HTMLImageElement>(".input-media-preview figure > img")
        ?.getAttribute("src"),
    ).toBe(snapshot.lens.input?.media[1]?.uri);
    const metadata = element.shadowRoot
      ?.querySelector(".input-media-metadata")
      ?.textContent?.replace(/\s+/g, " ");
    expect(metadata).toContain("media-node-000002");
    expect(metadata).toContain("node-000002");
    expect(metadata).toContain("700 × 800");
    expect(metadata).toContain("visible_subregion");
    expect(metadata).toContain("45, 50 · 100 × 120");
    expect(metadata).toContain("50, 60 · 70 × 80");
    expect(metadata).toContain("image/png");
    expect(thumbnails[1]?.getAttribute("aria-current")).toBe("true");
  });
});

describe("Lens Settings", () => {
  it("starts with explicitly named setting groups instead of a redundant visible header", async () => {
    window.history.replaceState({}, "", "/?view=settings&platform=macos");
    await import("./lens-app");
    const element = document.createElement("lens-app") as HTMLElement & {
      updateComplete: Promise<boolean>;
    };
    document.body.append(element);
    await element.updateComplete;

    const main = element.shadowRoot?.querySelector("main");
    const groups = Array.from(main?.children ?? []).filter((child) =>
      child.classList.contains("settings-group"),
    );

    expect(main?.getAttribute("aria-label")).toBe("Settings");
    expect(main?.querySelector("header")).toBeNull();
    expect(groups.map((group) => group.querySelector("h2")?.textContent)).toEqual([
      "AI Agent",
      "Agent Prompt",
      "Working Directory",
      "Accessibility",
    ]);
  });

  it("preserves native HTML behavior on the platform presentation targets", async () => {
    window.history.replaceState({}, "", "/?view=settings&platform=macos");
    await import("./lens-app");
    const element = document.createElement("lens-app") as HTMLElement & {
      updateComplete: Promise<boolean>;
    };
    document.body.append(element);
    await element.updateComplete;
    await vi.waitFor(() => {
      expect(element.shadowRoot?.querySelector<HTMLInputElement>(".directory-field")?.value).toBe(
        "/tmp",
      );
    });

    const directory = element.shadowRoot?.querySelector<HTMLInputElement>(".directory-field");
    const prompt = element.shadowRoot?.querySelector<HTMLTextAreaElement>(".prompt-editor");

    expect(directory).toBeInstanceOf(HTMLInputElement);
    expect(directory?.type).toBe("text");
    expect(directory?.readOnly).toBe(true);
    expect(prompt).toBeInstanceOf(HTMLTextAreaElement);
    expect(prompt?.required).toBe(true);
    expect(prompt?.disabled).toBe(false);
  });
});
