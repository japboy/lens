// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { AppView } from "./presentation-context";
import type { AppSnapshot } from "./types";

const operationId = "0198e6de-d046-7bf2-b8b2-d84cfaba7e2d";

const snapshot: AppSnapshot = {
  revision: 1,
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
      schema_version: 2,
      selection_id: operationId,
      targets: [
        {
          id: "macos:com.apple.Safari:417",
          identity: {
            window_id: 417,
            bundle_id: "com.apple.Safari",
            pid: 417,
          },
          facts_revision: 1,
          facts: {
            title: "Fixture",
            application_name: "Safari",
            frame: { x: 0, y: 0, width: 800, height: 600 },
          },
        },
        {
          id: "macos:com.apple.TextEdit:512",
          identity: {
            window_id: 512,
            bundle_id: "com.apple.TextEdit",
            pid: 512,
          },
          facts_revision: 1,
          facts: {
            title: "Notes",
            application_name: "TextEdit",
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
const closeCurrentWindow = vi.hoisted(() => vi.fn<() => Promise<void>>(async () => undefined));

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
  getCurrentWindow: () => ({ close: closeCurrentWindow }),
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
            window: {
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
            window: {
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
    const closeButton = overlayRoot?.querySelector('[aria-label="Stop Lens and close"]');
    expect(closeButton?.getAttribute("data-tauri-drag-region")).toBe("false");
    expect(closeButton?.getAttribute("aria-label")).toBe("Stop Lens and close");
    expect(closeButton?.getAttribute("title")).toBe("Stop Lens and close");
    expect(closeButton?.classList.contains("close-button")).toBe(true);
    expect(closeButton?.querySelector(".fa-xmark")?.getAttribute("aria-hidden")).toBe("true");
    expect(closeButton?.textContent?.trim()).toBe("");
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

  it("pairs Interpretation, Source, and Diagnostics with their keyboard-selected panels", async () => {
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
      "Interpretation",
      "Source",
      "Diagnostics",
    ]);
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");
    expect(tabs.map((tab) => tab.id)).toEqual([
      "interpretation-tab",
      "source-tab",
      "diagnostics-tab",
    ]);

    const expectSelectedPanel = (selectedIndex: number) => {
      const tab = tabs[selectedIndex];
      const panel = overlayRoot?.querySelector('[role="tabpanel"]');
      expect(panel?.id).toBe(tab?.getAttribute("aria-controls"));
      expect(panel?.getAttribute("aria-labelledby")).toBe(tab?.id);
      expect(overlayRoot?.querySelectorAll('[role="tabpanel"]')).toHaveLength(1);
      expect(tabs.map((item) => item.getAttribute("aria-selected"))).toEqual(
        tabs.map((_, index) => (index === selectedIndex ? "true" : "false")),
      );
      expect(tabs.map((item) => item.tabIndex)).toEqual(
        tabs.map((_, index) => (index === selectedIndex ? 0 : -1)),
      );
    };
    expectSelectedPanel(0);

    tabs[1]?.click();
    await overlayView?.updateComplete;
    expect(overlayRoot?.querySelector("#source-panel")).not.toBeNull();
    expect(overlayRoot?.querySelector("#diagnostics-panel")).toBeNull();
    expectSelectedPanel(1);

    tabs[2]?.click();
    await overlayView?.updateComplete;
    expect(overlayRoot?.querySelector("#diagnostics-panel .empty-state")?.textContent).toContain(
      "No extraction diagnostics are available.",
    );
    expect(overlayRoot?.querySelector("#source-panel")).toBeNull();
    expectSelectedPanel(2);

    tabs[2]?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await overlayView?.updateComplete;
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");
    expectSelectedPanel(0);
    expect(overlayRoot?.activeElement).toBe(tabs[0]);

    tabs[0]?.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await overlayView?.updateComplete;
    expect(tabs[2]?.getAttribute("aria-selected")).toBe("true");
    expectSelectedPanel(2);

    tabs[2]?.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
    await overlayView?.updateComplete;
    expectSelectedPanel(0);
    expect(overlayRoot?.activeElement).toBe(tabs[0]);

    tabs[0]?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    await overlayView?.updateComplete;
    expectSelectedPanel(2);
    expect(overlayRoot?.activeElement).toBe(tabs[2]);
  });

  it("presents ACP images in the Hero while preserving narrative block order", async () => {
    const element = await createLensApp("overlay");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-overlay-view")?.querySelector(".lens-output img"),
      ).not.toBeNull();
    });
    const overlayRoot = viewRoot(element, "lens-overlay-view");

    const output = overlayRoot?.querySelector(".lens-output");
    const blocks = Array.from(
      output?.querySelectorAll(".lens-output-narrative > lens-markdown") ?? [],
    );
    const image = output?.querySelector(".output-media-slide > img");

    expect(output?.firstElementChild?.tagName.toLowerCase()).toBe("lens-output-media");
    expect(blocks.map((block) => block.textContent?.trim())).toEqual([
      "Before image",
      "After image",
    ]);
    expect(output?.querySelectorAll(".output-media-slide")).toHaveLength(1);
    expect(output?.querySelector(".lens-output-narrative img")).toBeNull();
    expect(image?.getAttribute("src")).toBe("data:image/png;base64,iVBORw0KGgo=");
    expect(image?.getAttribute("alt")).toBe("Agent image 1 of 1");
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

    const selected = snapshot.lens.input?.media[1];
    const first = snapshot.lens.input?.media[0];
    if (!selected || !first || !snapshot.lens.input) throw new Error("Media fixture is incomplete");
    if (!gallery) throw new Error("Media gallery is missing");
    (gallery as typeof gallery & { lens: typeof snapshot.lens }).lens = {
      ...snapshot.lens,
      input: { ...snapshot.lens.input, media: [selected, first] },
    };
    await gallery.updateComplete;
    expect(
      overlayRoot
        ?.querySelector<HTMLImageElement>(".input-media-preview figure > img")
        ?.getAttribute("src"),
    ).toBe(selected.uri);
    expect(
      overlayRoot
        ?.querySelectorAll<HTMLButtonElement>(".input-media-thumbnail")[0]
        ?.getAttribute("aria-current"),
    ).toBe("true");
  });

  it("invokes finite monitoring controls and stops successfully before closing", async () => {
    snapshot.lens = {
      ...completedLens,
      live: {
        lifecycle: "watching",
        health: "healthy",
        freshness: "current",
        agent_refresh_interval_seconds: 180,
      },
    };
    const { invoke } = await import("@tauri-apps/api/core");
    const element = await createLensApp("overlay");
    await vi.waitFor(() => {
      expect(
        Array.from(viewRoot(element, "lens-overlay-view")?.querySelectorAll("button") ?? []).some(
          (button) => button.textContent?.trim() === "Pause Updates",
        ),
      ).toBe(true);
    });
    const overlayRoot = viewRoot(element, "lens-overlay-view");
    const pause = Array.from(overlayRoot?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "Pause Updates",
    );
    pause?.click();
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("pause_lens", { operationId });
    });

    overlayRoot?.querySelector<HTMLButtonElement>('[aria-label="Stop Lens and close"]')?.click();
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("stop_lens", { operationId });
      expect(closeCurrentWindow).toHaveBeenCalledOnce();
    });
    const stopCallIndex = vi
      .mocked(invoke)
      .mock.calls.findIndex(([command]) => command === "stop_lens");
    expect(stopCallIndex).toBeGreaterThanOrEqual(0);
    expect(vi.mocked(invoke).mock.invocationCallOrder[stopCallIndex]).toBeLessThan(
      closeCurrentWindow.mock.invocationCallOrder[0] ?? Number.POSITIVE_INFINITY,
    );
  });

  it("resumes a paused live operation", async () => {
    snapshot.lens = {
      ...completedLens,
      live: {
        lifecycle: "paused",
        health: "healthy",
        freshness: "unverified",
        agent_refresh_interval_seconds: 180,
      },
    };
    const { invoke } = await import("@tauri-apps/api/core");
    const element = await createLensApp("overlay");
    await vi.waitFor(() => {
      expect(
        Array.from(viewRoot(element, "lens-overlay-view")?.querySelectorAll("button") ?? []).some(
          (button) => button.textContent?.trim() === "Resume Updates",
        ),
      ).toBe(true);
    });
    const resume = Array.from(
      viewRoot(element, "lens-overlay-view")?.querySelectorAll("button") ?? [],
    ).find((button) => button.textContent?.trim() === "Resume Updates");
    resume?.click();
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("resume_lens", { operationId });
    });
  });

  it("keeps the window open when Stop fails", async () => {
    const element = await createLensApp("overlay");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-overlay-view")?.querySelector('[aria-label="Stop Lens and close"]'),
      ).not.toBeNull();
    });
    const { invoke } = await import("@tauri-apps/api/core");
    const mockedInvoke = vi.mocked(invoke);
    const previousImplementation = mockedInvoke.getMockImplementation();
    mockedInvoke.mockImplementation(async (command, ...arguments_) => {
      if (command === "stop_lens") throw new Error("Stop failed");
      return previousImplementation?.(command, ...arguments_);
    });

    try {
      viewRoot(element, "lens-overlay-view")
        ?.querySelector<HTMLButtonElement>('[aria-label="Stop Lens and close"]')
        ?.click();
      await vi.waitFor(() => {
        expect(mockedInvoke).toHaveBeenCalledWith("stop_lens", { operationId });
        expect(
          viewRoot(element, "lens-overlay-view")?.querySelector('[role="alert"]')?.textContent,
        ).toContain("Stop failed");
      });
      expect(closeCurrentWindow).not.toHaveBeenCalled();
    } finally {
      if (previousImplementation) mockedInvoke.mockImplementation(previousImplementation);
    }
  });
});

describe("Lens Settings", () => {
  it("uses one System Settings-style navigation authority and one detail destination", async () => {
    const element = await createLensApp("settings");
    const settingsRoot = viewRoot(element, "lens-settings-view");

    const main = settingsRoot?.querySelector("main");
    const sidebar = settingsRoot?.querySelector(".settings-sidebar");
    const navigation = settingsRoot?.querySelector('nav[aria-label="Settings sections"]');
    const globalStatus = settingsRoot?.querySelector(".settings-sidebar-status [role='status']");
    const items = Array.from(navigation?.querySelectorAll<HTMLButtonElement>("button") ?? []);

    expect(main?.getAttribute("aria-label")).toBe("Settings");
    expect(sidebar?.contains(globalStatus ?? null)).toBe(true);
    await vi.waitFor(() => {
      expect(globalStatus?.textContent?.trim()).toBe("Transformation complete");
    });
    expect(globalStatus?.getAttribute("aria-labelledby")).toBe("lens-status-label");
    expect(settingsRoot?.querySelector(".settings-footer")).toBeNull();
    expect(settingsRoot?.querySelector(".settings-compact-navigation")).toBeNull();
    expect(settingsRoot?.querySelector("#settings-destination")).toBeNull();
    expect(items.map((item) => item.textContent?.trim())).toEqual(["General", "Agent Prompt"]);
    expect(items[0]?.getAttribute("aria-current")).toBe("page");
    expect(
      settingsRoot?.querySelector(".settings-detail-panel:not([hidden]) h1")?.textContent,
    ).toBe("General");

    items.find((item) => item.textContent?.trim() === "Agent Prompt")?.click();
    await vi.waitFor(() => {
      expect(
        settingsRoot?.querySelector(".settings-detail-panel:not([hidden]) h1")?.textContent?.trim(),
      ).toBe("Agent Prompt");
    });
    expect(
      items
        .find((item) => item.textContent?.trim() === "Agent Prompt")
        ?.getAttribute("aria-current"),
    ).toBe("page");
    expect(
      settingsRoot?.querySelector(".prompt-composition-result strong")?.textContent?.trim(),
    ).toBe("Rendered Prompt");
  });

  it("keeps actionable Settings failures in their owning detail pane", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const mockedInvoke = vi.mocked(invoke);
    const previousImplementation = mockedInvoke.getMockImplementation();
    mockedInvoke.mockImplementation(async (command, ...arguments_) => {
      if (command === "set_agent") throw new Error("Agent update failed");
      return previousImplementation?.(command, ...arguments_);
    });

    try {
      const element = await createLensApp("settings");
      const settingsRoot = viewRoot(element, "lens-settings-view");
      await vi.waitFor(() => {
        expect(
          settingsRoot?.querySelector<HTMLInputElement>('input[value="claude"]'),
        ).not.toBeNull();
      });

      settingsRoot?.querySelector<HTMLInputElement>('input[value="claude"]')?.click();

      await vi.waitFor(() => {
        const feedback = settingsRoot?.querySelector(
          ".settings-detail-panel:not([hidden]) .settings-context-feedback[role='alert']",
        );
        expect(feedback?.textContent).toContain("Agent update failed");
      });
      expect(settingsRoot?.querySelector(".settings-sidebar-status")?.textContent).not.toContain(
        "Agent update failed",
      );

      Array.from(settingsRoot?.querySelectorAll<HTMLButtonElement>(".settings-nav-item") ?? [])
        .find((button) => button.textContent?.trim() === "Agent Prompt")
        ?.click();
      await vi.waitFor(() => {
        const visiblePanel = settingsRoot?.querySelector(".settings-detail-panel:not([hidden])");
        expect(visiblePanel?.querySelector("h1")?.textContent?.trim()).toBe("Agent Prompt");
        expect(visiblePanel?.querySelector(".settings-context-feedback[role='alert']")).toBeNull();
      });
    } finally {
      if (previousImplementation) mockedInvoke.mockImplementation(previousImplementation);
    }
  });

  it("preserves native controls and saves the complete prompt template atomically", async () => {
    const element = await createLensApp("settings");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-settings-view")?.querySelector<HTMLInputElement>(".directory-field")
          ?.value,
      ).toBe("/tmp");
    });
    const settingsRoot = viewRoot(element, "lens-settings-view");
    const agentPrompt = Array.from(
      settingsRoot?.querySelectorAll<HTMLButtonElement>(".settings-nav-item") ?? [],
    ).find((button) => button.textContent?.trim() === "Agent Prompt");
    agentPrompt?.click();
    await vi.waitFor(() => {
      expect(settingsRoot?.querySelector(".prompt-editor")).not.toBeNull();
    });

    const directory = settingsRoot?.querySelector<HTMLInputElement>(".directory-field");
    const prompt = settingsRoot?.querySelector<HTMLTextAreaElement>(".prompt-editor");

    expect(directory).toBeInstanceOf(HTMLInputElement);
    expect(directory?.type).toBe("text");
    expect(directory?.readOnly).toBe(true);
    expect(prompt).toBeInstanceOf(HTMLTextAreaElement);
    expect(prompt?.required).toBe(true);
    expect(prompt?.disabled).toBe(false);
    if (!prompt) throw new Error("Prompt editor is missing");
    prompt.value = "Updated instruction.\n\n{turn_instruction}";
    prompt.dispatchEvent(new Event("input", { bubbles: true }));
    await element.updateComplete;
    settingsRoot?.querySelector<HTMLButtonElement>('button[type="submit"]')?.click();

    const { invoke } = await import("@tauri-apps/api/core");
    await vi.waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("set_agent_prompt_template", {
        agentPromptTemplate: {
          ...snapshot.config.agent_prompt_template,
          common: "Updated instruction.\n\n{turn_instruction}",
        },
      });
    });
    await vi.waitFor(() => {
      const feedback = settingsRoot?.querySelector(
        "lens-prompt-settings .settings-context-feedback[role='status']",
      );
      expect(feedback?.textContent).toContain("Agent prompt template updated.");
    });
    expect(
      settingsRoot?.querySelector(".settings-sidebar-status [role='status']")?.textContent?.trim(),
    ).toBe("Transformation complete");
  });
});

it("correlates notification submissions, rejects duplicate clicks, and permits retry after failure", async () => {
  const { invoke } = await import("@tauri-apps/api/core");
  const element = await createLensApp("overlay");
  await vi.waitFor(() => {
    const view = element.shadowRoot!.querySelector("lens-overlay-view") as HTMLElement & {
      model: { lens: { operation_id?: string } };
    };
    expect(view.model.lens.operation_id).toBe(operationId);
  });
  let rejectResponse: (reason: Error) => void = () => undefined;
  vi.mocked(invoke).mockImplementationOnce(
    () =>
      new Promise((_, reject) => {
        rejectResponse = reject;
      }),
  );
  const intent = {
    type: "respond-interaction",
    instanceId: "instance",
    interactionId: "request",
    response: { action: "select", option_id: "allow" },
  };
  const send = () =>
    element
      .shadowRoot!.querySelector("lens-overlay-view")!
      .dispatchEvent(
        new CustomEvent("lens-overlay-intent", { detail: intent, bubbles: true, composed: true }),
      );
  send();
  send();
  expect(
    vi.mocked(invoke).mock.calls.filter(([name]) => name === "respond_agent_interaction"),
  ).toHaveLength(1);
  rejectResponse(new Error("Transport failed"));
  await vi.waitFor(() => {
    const view = element.shadowRoot!.querySelector("lens-overlay-view") as HTMLElement & {
      model: { interactionSubmission?: { stage: string } };
    };
    expect(view.model.interactionSubmission?.stage).toBe("failed");
  });
  send();
  await vi.waitFor(() =>
    expect(
      vi.mocked(invoke).mock.calls.filter(([name]) => name === "respond_agent_interaction"),
    ).toHaveLength(2),
  );
});
