// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { AppView } from "./presentation-context";
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

interface TestLensApp extends HTMLElement {
  context: { view: AppView; platform: "macos" };
  updateComplete: Promise<boolean>;
}

async function createLensApp(view: AppView): Promise<TestLensApp> {
  window.history.replaceState({}, "", `/?view=${view}&platform=macos`);
  await import("./lens-app");
  const element = document.createElement("lens-app") as TestLensApp;
  element.context = { view, platform: "macos" };
  document.body.append(element);
  await element.updateComplete;
  return element;
}

function viewRoot(element: TestLensApp, selector: string): ShadowRoot | undefined {
  return element.shadowRoot?.querySelector<HTMLElement>(selector)?.shadowRoot ?? undefined;
}

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
    const { invoke } = await import("@tauri-apps/api/core");
    const element = await createLensApp("target-selection");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-target-selection-view")?.querySelectorAll(".target-selection-card"),
      ).toHaveLength(2);
    });
    const selectionRoot = viewRoot(element, "lens-target-selection-view");

    const images = Array.from(
      selectionRoot?.querySelectorAll<HTMLImageElement>(".target-selection-image > img") ?? [],
    );
    expect(images.map((image) => image.getAttribute("src"))).toEqual([
      `lens://selection/${operationId}/window/417`,
      `lens://selection/${operationId}/window/512`,
    ]);
    expect(selectionRoot?.querySelector(".overlay-header")).toBeNull();
    expect(selectionRoot?.querySelector("[data-tauri-drag-region]")).toBeNull();
    expect(selectionRoot?.querySelector('[aria-label="Close Lens"]')).toBeNull();
    expect(
      selectionRoot?.querySelector(".target-selection-shell")?.hasAttribute("data-entrance"),
    ).toBe(false);
    expect(selectionRoot?.querySelector(".target-selection-count")?.textContent).toContain("2 / 4");

    selectionRoot?.querySelector<HTMLButtonElement>('[aria-label="Add another window"]')?.click();
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("add_lens_target", { operationId });
    });
    const removeButton = selectionRoot?.querySelector<HTMLButtonElement>(
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
    const confirmButton = selectionRoot?.querySelector<HTMLButtonElement>(
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
    const element = await createLensApp("overlay");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-overlay-view")?.querySelector(".overlay-source-count")?.textContent,
      ).toContain("2 selected windows");
    });
    const overlayRoot = viewRoot(element, "lens-overlay-view");

    expect(
      overlayRoot?.querySelector(".overlay-header")?.getAttribute("data-tauri-drag-region"),
    ).toBe("deep");
    expect(
      overlayRoot?.querySelector(".close-button")?.getAttribute("data-tauri-drag-region"),
    ).toBe("false");
    expect(overlayRoot?.querySelector(".overlay-app-icon")?.getAttribute("src")).toBeTruthy();
    expect(overlayRoot?.querySelector(".overlay-app-mark")).toBeNull();
    expect(
      overlayRoot?.querySelector(".overlay-title")?.classList.contains("visually-hidden"),
    ).toBe(true);
    expect(overlayRoot?.querySelector(".overlay-source-count")?.textContent).toContain(
      "2 selected windows",
    );
    expect(overlayRoot?.querySelector(".overlay-source-targets")?.getAttribute("title")).toContain(
      "TextEdit — Notes",
    );
  });

  it("exposes Source and Diagnostics as ordered keyboard tabs", async () => {
    const element = await createLensApp("overlay");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-overlay-view")?.querySelector(".overlay-source-count")?.textContent,
      ).toContain("2 selected windows");
    });
    const overlayView = element.shadowRoot?.querySelector<
      HTMLElement & { updateComplete: Promise<boolean> }
    >("lens-overlay-view");
    const overlayRoot = overlayView?.shadowRoot;
    const tabs = Array.from(overlayRoot?.querySelectorAll<HTMLButtonElement>('[role="tab"]') ?? []);

    expect(tabs.map((tab) => tab.textContent?.trim())).toEqual([
      "Translation",
      "Source",
      "Diagnostics",
    ]);
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");

    tabs[1]?.click();
    await overlayView?.updateComplete;
    expect(overlayRoot?.querySelector("#source-panel")).not.toBeNull();
    expect(overlayRoot?.querySelector("#diagnostics-panel")).toBeNull();

    tabs[2]?.click();
    await overlayView?.updateComplete;
    expect(overlayRoot?.querySelector("#diagnostics-panel .empty-state")?.textContent).toContain(
      "No extraction diagnostics are available.",
    );
    expect(overlayRoot?.querySelector("#source-panel")).toBeNull();

    tabs[2]?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await overlayView?.updateComplete;
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");

    tabs[0]?.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await overlayView?.updateComplete;
    expect(tabs[2]?.getAttribute("aria-selected")).toBe("true");
  });

  it("renders ACP image data inline and preserves surrounding block order", async () => {
    const element = await createLensApp("overlay");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-overlay-view")?.querySelector(".lens-output img"),
      ).not.toBeNull();
    });
    const overlayRoot = viewRoot(element, "lens-overlay-view");

    const output = overlayRoot?.querySelector(".lens-output");
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
    const element = await createLensApp("overlay");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-overlay-view")?.querySelector(".overlay-source-count")?.textContent,
      ).toContain("2 selected windows");
    });
    const overlayView = element.shadowRoot?.querySelector<
      HTMLElement & { updateComplete: Promise<boolean> }
    >("lens-overlay-view");
    const overlayRoot = overlayView?.shadowRoot;
    overlayRoot?.querySelector<HTMLButtonElement>("#source-tab")?.click();
    await overlayView?.updateComplete;
    await vi.waitFor(() => {
      expect(
        overlayRoot?.querySelector<HTMLImageElement>(".input-media-preview figure > img"),
      ).not.toBeNull();
    });

    const preview = overlayRoot?.querySelector<HTMLImageElement>(
      ".input-media-preview figure > img",
    );
    const thumbnails = Array.from(
      overlayRoot?.querySelectorAll<HTMLButtonElement>(".input-media-thumbnail") ?? [],
    );
    const source = overlayRoot?.querySelector(".source-content code")?.textContent;
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
    const gallery = overlayRoot?.querySelector<HTMLElement & { updateComplete: Promise<boolean> }>(
      "lens-media-gallery",
    );
    await gallery?.updateComplete;
    expect(
      overlayRoot
        ?.querySelector<HTMLImageElement>(".input-media-preview figure > img")
        ?.getAttribute("src"),
    ).toBe(snapshot.lens.input?.media[1]?.uri);
    const metadata = overlayRoot
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
    const element = await createLensApp("settings");
    const settingsRoot = viewRoot(element, "lens-settings-view");

    const main = settingsRoot?.querySelector("main");
    const groups = Array.from(main?.querySelectorAll(".settings-group") ?? []);

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
    const element = await createLensApp("settings");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-settings-view")?.querySelector<HTMLInputElement>(".directory-field")
          ?.value,
      ).toBe("/tmp");
    });
    const settingsRoot = viewRoot(element, "lens-settings-view");

    const directory = settingsRoot?.querySelector<HTMLInputElement>(".directory-field");
    const prompt = settingsRoot?.querySelector<HTMLTextAreaElement>(".prompt-editor");

    expect(directory).toBeInstanceOf(HTMLInputElement);
    expect(directory?.type).toBe("text");
    expect(directory?.readOnly).toBe(true);
    expect(prompt).toBeInstanceOf(HTMLTextAreaElement);
    expect(prompt?.required).toBe(true);
    expect(prompt?.disabled).toBe(false);
  });
});
