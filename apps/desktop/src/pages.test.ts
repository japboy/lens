// @vitest-environment jsdom

import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { installGeneratedPage } from "./rendering/generated-page.test-helper";
import { startPage } from "./entries/start-page";
import type { LensSelect } from "./components/lens-select";
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
    prompt_presets: {
      schema_version: 2,
      execution_revision: 1,
      revision: 1,
      selected_id: "conceptual",
      presets: [
        {
          id: "conceptual",
          name: "Conceptual",

          revision: 1,
          template: {
            schema_version: 1,
            common: "Transform the selected content.\n\n{turn_instruction}",
            full_projection: "Use the initial projection.",
            source_checkpoint: "Replace revision {base_revision} with {target_revision}.",
            current_projection_retry: "Retry revision {applied_revision}.",
          },
        },
      ],
    },
  },
  agent_selection: {
    stage: "selected",
    candidate: "codex",
    supports_logout: false,
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
    prompt_execution_revision: 1,
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
    if (command === "get_about_info")
      return {
        name: "Lens",
        version: "0.1.0",
        copyright: "Copyright © 2026 Yu Inao",
      };
    if (command === "get_about_documents")
      return {
        license: "Apache text\n<not-markup>",
        notice: "Original project by Yu Inao",
      };
    if (command === "get_app_snapshot") return snapshot;
    if (command === "get_session_view") return { revision: 0, phase: "idle" };
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

interface TestPage extends HTMLElement {
  readonly updateComplete: Promise<boolean>;
}

async function createPage(view: AppView): Promise<TestPage> {
  window.history.replaceState({}, "", `/${view}.html?platform=macos`);
  installGeneratedPage(view);
  const loaders = {
    about: () => import("./pages/about-page"),
    settings: () => import("./pages/settings-page"),
    "settings-recovery": () => import("./pages/settings-recovery-page"),
    overlay: () => import("./pages/overlay-page"),
    "target-selection": () => import("./pages/target-selection-page"),
  };
  await startPage(view, loaders[view]);
  const element = document.querySelector(`lens-${view}-page`) as TestPage;
  await element.updateComplete;
  return element;
}

function viewRoot(element: TestPage, selector: string): ShadowRoot | HTMLElement | undefined {
  return element.querySelector<HTMLElement>(selector)?.shadowRoot ?? undefined;
}

describe("progressive DSD resources", () => {
  it("keeps Settings navigation and permission independent from the first snapshot, then preserves a draft across publication", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");
    const original = vi.mocked(invoke).getMockImplementation()!;
    let resolveSnapshot!: (value: AppSnapshot) => void;
    const pending = new Promise<AppSnapshot>((resolve) => {
      resolveSnapshot = resolve;
    });
    vi.mocked(invoke).mockImplementation((command) =>
      command === "get_app_snapshot" ? pending : original(command),
    );
    try {
      const page = await createPage("settings");
      const root = viewRoot(page, "lens-settings-view")!;
      expect(root.querySelector("#agent-heading")?.textContent).toBe("AI Agent");
      expect(root.querySelector(".settings-sidebar-status-value")?.textContent?.trim()).toBe("");
      await vi.waitFor(() =>
        expect(root.querySelector(".permission-row output")?.textContent).toContain("Allowed"),
      );
      const navigation = Array.from(root.querySelectorAll<HTMLButtonElement>(".settings-nav-item"));
      expect(navigation.every((button) => !button.disabled)).toBe(true);
      navigation.find((button) => button.textContent?.trim() === "Prompt Presets")!.click();
      await vi.waitFor(() =>
        expect(root.querySelector("#agent-prompt-heading")?.closest("[hidden]")).toBeNull(),
      );
      const prompt = root.querySelector("lens-prompt-settings")!;
      resolveSnapshot(snapshot);
      await vi.waitFor(() => expect(prompt.querySelector("textarea")).not.toBeNull());
      const editor = prompt.querySelector("textarea")!;
      editor.value = "Unsaved progressive draft\\n{turn_instruction}";
      editor.dispatchEvent(new Event("input", { bubbles: true }));
      const listener = vi
        .mocked(listen)
        .mock.calls.find(([event]) => event === "app-state-changed")?.[1];
      listener?.({ event: "app-state-changed", id: 1, payload: { ...snapshot, revision: 2 } });
      await page.updateComplete;
      await (
        page.querySelector(
          "lens-settings-view",
        ) as import("./components/lens-settings-view").LensSettingsView
      ).updateComplete;
      await (prompt as import("./components/lens-prompt-settings").LensPromptSettings)
        .updateComplete;
      expect(root.querySelector("lens-prompt-settings")).toBe(prompt);
      expect(prompt.querySelector("textarea")).toBe(editor);
      expect(editor.value).toBe("Unsaved progressive draft\\n{turn_instruction}");
    } finally {
      vi.mocked(invoke).mockImplementation(original);
    }
  });

  it("contains first-snapshot failure while the Settings shell and About action remain available", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const original = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation((command) =>
      command === "get_app_snapshot"
        ? Promise.reject(new Error("Snapshot unavailable"))
        : original(command),
    );
    try {
      const page = await createPage("settings");
      const root = viewRoot(page, "lens-settings-view")!;
      await vi.waitFor(() =>
        expect(root.querySelector("[role=alert]")?.textContent).toContain("Snapshot unavailable"),
      );
      expect(root.querySelectorAll("[role=alert]")).toHaveLength(1);
      expect(root.querySelector(".settings-sidebar-status-value")?.textContent?.trim()).toBe("");
      const about = Array.from(root.querySelectorAll<HTMLButtonElement>("button")).find(
        (button) => button.textContent?.trim() === "About",
      )!;
      expect(about.disabled).toBe(false);
      about.click();
      await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("show_about"));
    } finally {
      vi.mocked(invoke).mockImplementation(original);
    }
  });
});

describe("About", () => {
  it("loads embedded documents without snapshots or permission checks, and switches read-only text", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");
    const element = await createPage("about");
    await vi.waitFor(() =>
      expect(
        viewRoot(element, "lens-about-view")
          ?.querySelector(".document-text")
          ?.getAttribute("aria-busy"),
      ).toBe("false"),
    );
    const root = viewRoot(element, "lens-about-view")!;
    const documentText = root.querySelector<HTMLElement>(".document-text")!;
    expect(documentText.textContent).toBe("Apache text\n<not-markup>");
    expect(documentText.isContentEditable).not.toBe(true);
    expect(documentText.tabIndex).toBe(0);
    expect(root.querySelector("not-markup")).toBeNull();
    expect(listen).not.toHaveBeenCalled();
    expect(vi.mocked(invoke).mock.calls.map(([name]) => name)).toEqual([
      "get_about_info",
      "get_about_documents",
    ]);
    const select = root.querySelector<LensSelect>("lens-select")!;
    documentText.scrollTop = 100;
    select.value = "notice";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() =>
      expect(root.querySelector<HTMLElement>(".document-text")?.textContent).toBe(
        "Original project by Yu Inao",
      ),
    );
    expect(root.querySelector<HTMLElement>(".document-text")).toBe(documentText);
    expect(root.querySelector<HTMLElement>(".document-text")?.scrollTop).toBe(0);
  });

  it("preserves line endings, blank lines and literal text across document blocks", async () => {
    const element = await createPage("about");
    await vi.waitFor(() =>
      expect(
        viewRoot(element, "lens-about-view")
          ?.querySelector(".document-text")
          ?.getAttribute("aria-busy"),
      ).toBe("false"),
    );
    const view = viewRoot(element, "lens-about-view")!.querySelector(
      "lens-license-document",
    ) as import("./components/lens-license-document").LensLicenseDocument;
    const license =
      Array.from({ length: 100 }, (_, index) =>
        index % 3 === 0 ? "\r\n" : `Line ${index} <not-markup>\n`,
      ).join("") + "Final line";
    const aboutView = element.querySelector(
      "lens-about-view",
    ) as import("./components/lens-about-view").LensAboutView;
    aboutView.documents = { stage: "ready", value: { license, notice: "" } };
    await aboutView.updateComplete;
    await view.updateComplete;
    const root = viewRoot(element, "lens-about-view")!;
    expect(root.querySelectorAll(".document-chunk").length).toBeGreaterThan(1);
    expect(root.querySelector(".document-text")?.textContent).toBe(license);
    expect(root.querySelector("not-markup")).toBeNull();
    const select = root.querySelector<LensSelect>("lens-select")!;
    select.value = "notice";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await view.updateComplete;
    await vi.waitFor(() => expect(root.querySelector(".document-text")?.textContent).toBe(""));
  });

  it("keeps the shell and metadata visible while documents are pending and preserves the pending selection", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    let resolveInfo!: (value: unknown) => void;
    let resolveDocuments!: (value: unknown) => void;
    const info = new Promise<unknown>((resolve) => {
      resolveInfo = resolve;
    });
    const documents = new Promise<unknown>((resolve) => {
      resolveDocuments = resolve;
    });
    vi.mocked(invoke)
      .mockImplementationOnce(() => info)
      .mockImplementationOnce(() => documents);
    const element = await createPage("about");
    await vi.waitFor(() =>
      expect(
        viewRoot(element, "lens-about-view")?.querySelector<LensSelect>("lens-select"),
      ).toBeTruthy(),
    );
    const root = viewRoot(element, "lens-about-view")!;
    const header = root.querySelector("header")!;
    expect(header.querySelector("h1")?.textContent).toBe("Lens");
    expect(root.textContent).not.toContain("Loading About");
    expect(root.querySelector(".document-text")?.getAttribute("aria-busy")).toBe("true");
    expect(invoke).toHaveBeenCalledWith("get_about_documents");
    resolveInfo({ name: "Lens", version: "1.2.3", copyright: "Copyright" });
    await vi.waitFor(() => expect(header.textContent).toContain("Version 1.2.3"));
    await vi.waitFor(() => expect(root.querySelector(".document-text")?.textContent).toBe(""));
    const select = root.querySelector<LensSelect>("lens-select")!;
    select.value = "notice";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() =>
      expect(root.querySelector(".document-text")?.getAttribute("aria-label")).toBe("NOTICE"),
    );
    const region = root.querySelector(".document-text");
    resolveDocuments({ license: "License", notice: "Notice" });
    await vi.waitFor(() => expect(region?.textContent).toBe("Notice"));
    expect(root.querySelector("header")).toBe(header);
    expect(root.querySelector(".document-text")).toBe(region);
    expect(region?.getAttribute("aria-busy")).toBe("false");
  });

  it("contains document failures within the document region", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    vi.mocked(invoke)
      .mockResolvedValueOnce({ name: "Lens", version: "1.2.3", copyright: "Copyright" })
      .mockRejectedValueOnce(new Error("Document unavailable"));
    const element = await createPage("about");
    await vi.waitFor(() =>
      expect(
        viewRoot(element, "lens-about-view")?.querySelector(".document-text [role=alert]")
          ?.textContent,
      ).toContain("Document unavailable"),
    );
    const root = viewRoot(element, "lens-about-view")!;
    await vi.waitFor(() =>
      expect(root.querySelector("header")?.textContent).toContain("Version 1.2.3"),
    );
    expect(root.querySelector<LensSelect>("lens-select")?.disabled).toBe(false);
    expect(root.querySelector(".document-text")?.getAttribute("aria-busy")).toBe("false");
  });

  it("still loads documents when app metadata fails", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    vi.mocked(invoke).mockRejectedValueOnce(new Error("Metadata unavailable"));
    const element = await createPage("about");
    await vi.waitFor(() =>
      expect(
        viewRoot(element, "lens-about-view")?.querySelector(".document-text")?.textContent,
      ).toBe("Apache text\n<not-markup>"),
    );
    const root = viewRoot(element, "lens-about-view")!;
    expect(root.querySelector("header [role=alert]")?.textContent).toContain(
      "Metadata unavailable",
    );
  });

  it("opens About from Settings without changing the selected page", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const element = await createPage("settings");
    await vi.waitFor(() =>
      expect(
        viewRoot(element, "lens-settings-view")?.querySelector(".about-entry button"),
      ).toBeTruthy(),
    );
    const root = viewRoot(element, "lens-settings-view")!;
    const prompt = [...root.querySelectorAll<HTMLButtonElement>(".settings-nav-item")].find(
      (button) => button.textContent?.includes("Prompt Presets"),
    )!;
    prompt.click();
    await vi.waitFor(() => expect(prompt.getAttribute("aria-current")).toBe("page"));
    root.querySelector<HTMLButtonElement>(".about-entry button")!.click();
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith("show_about"));
    expect(prompt.getAttribute("aria-current")).toBe("page");
  });
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
      prompt_execution_revision: 1,
      output_blocks: [],
    };
    const { invoke } = await import("@tauri-apps/api/core");
    const element = await createPage("target-selection");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-target-selection-view")?.querySelectorAll(".target-selection-card"),
      ).toHaveLength(2);
    });
    const selectionRoot = viewRoot(element, "lens-target-selection-view");

    await vi.waitFor(() =>
      expect(selectionRoot?.querySelectorAll(".target-selection-card img")).toHaveLength(2),
    );
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
    const element = await createPage("overlay");
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

  it("pairs content tabs with their keyboard-selected panels", async () => {
    const element = await createPage("overlay");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-overlay-view")?.querySelector(".overlay-source-count")?.textContent,
      ).toContain("2 selected windows");
    });
    const overlayView = element.querySelector<HTMLElement & { updateComplete: Promise<boolean> }>(
      "lens-overlay-view",
    );
    const overlayRoot = overlayView?.shadowRoot;
    const tabs = Array.from(overlayRoot?.querySelectorAll<HTMLButtonElement>('[role="tab"]') ?? []);

    expect(tabs.map((tab) => tab.textContent?.trim())).toEqual([
      "Interpretation",
      "Conversation",
      "Source",
      "Diagnostics",
    ]);
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");
    expect(tabs.map((tab) => tab.id)).toEqual([
      "interpretation-tab",
      "conversation-tab",
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
    expect(overlayRoot?.querySelector("lens-session-document")).not.toBeNull();
    expectSelectedPanel(1);

    tabs[1]?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await overlayView?.updateComplete;
    expect(overlayRoot?.querySelector("#source-panel")).not.toBeNull();
    expect(overlayRoot?.querySelector("#diagnostics-panel")).toBeNull();
    expectSelectedPanel(2);
    expect(overlayRoot?.activeElement).toBe(tabs[2]);

    tabs[3]?.click();
    await overlayView?.updateComplete;
    expect(overlayRoot?.querySelector("#diagnostics-panel .empty-state")?.textContent).toContain(
      "No extraction diagnostics are available.",
    );
    expect(overlayRoot?.querySelector("#source-panel")).toBeNull();
    expectSelectedPanel(3);

    tabs[3]?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await overlayView?.updateComplete;
    expectSelectedPanel(0);
    tabs[0]?.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true }));
    await overlayView?.updateComplete;
    expectSelectedPanel(3);
    tabs[3]?.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
    await overlayView?.updateComplete;
    expectSelectedPanel(0);
    tabs[0]?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    await overlayView?.updateComplete;
    expectSelectedPanel(3);
    expect(overlayRoot?.activeElement).toBe(tabs[3]);
  });

  it("presents ACP images in the Hero while preserving narrative block order", async () => {
    const element = await createPage("overlay");
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
    const element = await createPage("overlay");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-overlay-view")?.querySelector(".overlay-source-count")?.textContent,
      ).toContain("2 selected windows");
    });
    const overlayView = element.querySelector<HTMLElement & { updateComplete: Promise<boolean> }>(
      "lens-overlay-view",
    );
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
    const element = await createPage("overlay");
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
    const element = await createPage("overlay");
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
    const element = await createPage("overlay");
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
  it("reports screen recording settings launch failures without an application snapshot", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const mockedInvoke = vi.mocked(invoke);
    const original = mockedInvoke.getMockImplementation()!;
    mockedInvoke.mockImplementation(async (command, ...arguments_) => {
      if (command === "get_app_snapshot") throw new Error("Snapshot unavailable");
      if (command === "open_screen_recording_settings")
        throw new Error("System Settings unavailable");
      return original(command, ...arguments_);
    });
    try {
      const page = await createPage("settings");
      const root = viewRoot(page, "lens-settings-view")!;
      Array.from(root.querySelectorAll<HTMLButtonElement>("nav button"))
        .find((button) => button.textContent?.trim() === "Privacy & Security")!
        .click();
      await vi.waitFor(() => {
        expect(
          root.querySelector(".settings-detail-panel:not([hidden]) h1")?.textContent?.trim(),
        ).toBe("Privacy & Security");
      });
      expect(root.querySelectorAll("[role=alert]")).toHaveLength(1);
      expect(root.querySelector("[role=alert]")?.textContent).toContain("Snapshot unavailable");
      const button = root.querySelector<HTMLButtonElement>(
        '[aria-labelledby="screen-recording-heading"] button',
      )!;
      expect(button.disabled).toBe(false);
      button.click();
      await vi.waitFor(() => {
        expect(mockedInvoke).toHaveBeenCalledWith("open_screen_recording_settings");
        expect(
          root.querySelector(".settings-detail-panel:not([hidden]) [role=alert]")?.textContent,
        ).toContain("System Settings unavailable");
        const alerts = Array.from(root.querySelectorAll("[role=alert]"));
        expect(alerts).toHaveLength(2);
        expect(
          alerts.filter((alert) => alert.textContent?.includes("Snapshot unavailable")),
        ).toHaveLength(1);
        expect(
          alerts.filter((alert) => alert.textContent?.includes("System Settings unavailable")),
        ).toHaveLength(1);
        expect(button.disabled).toBe(false);
      });
    } finally {
      mockedInvoke.mockImplementation(original);
    }
  });

  it("opens screen recording settings explicitly and keeps launch errors in Privacy & Security", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const mockedInvoke = vi.mocked(invoke);
    const previousImplementation = mockedInvoke.getMockImplementation();
    mockedInvoke.mockImplementation(async (command, ...arguments_) => {
      if (command === "open_screen_recording_settings")
        throw new Error("System Settings unavailable");
      return previousImplementation?.(command, ...arguments_);
    });
    try {
      const element = await createPage("settings");
      const root = viewRoot(element, "lens-settings-view")!;
      const navigation = Array.from(root.querySelectorAll<HTMLButtonElement>("nav button"));
      navigation.find((button) => button.textContent?.trim() === "Privacy & Security")!.click();
      await vi.waitFor(() => {
        expect(
          root.querySelector(".settings-detail-panel:not([hidden]) h1")?.textContent?.trim(),
        ).toBe("Privacy & Security");
      });
      const section = root.querySelector('[aria-labelledby="screen-recording-heading"]')!;
      expect(section.textContent).toContain("Screen & System Audio Recording");
      expect(section.textContent).toContain("It does not record audio.");
      expect(section.querySelector("output")).toBeNull();
      expect(mockedInvoke).not.toHaveBeenCalledWith("open_screen_recording_settings");
      const button = section.querySelector<HTMLButtonElement>("button")!;
      button.focus();
      button.click();
      await vi.waitFor(() => {
        expect(mockedInvoke).toHaveBeenCalledWith("open_screen_recording_settings");
        expect(root.querySelector(".settings-detail-panel:not([hidden])")?.textContent).toContain(
          "System Settings unavailable",
        );
      });
      expect(section.querySelector("button")).toBe(button);
      expect(root instanceof ShadowRoot ? root.activeElement : document.activeElement).toBe(button);
      expect(button.disabled).toBe(false);
      mockedInvoke.mockImplementation(async (command, ...arguments_) => {
        if (command === "open_screen_recording_settings") return undefined;
        return previousImplementation?.(command, ...arguments_);
      });
      button.click();
      await vi.waitFor(() => {
        expect(root.querySelector(".settings-detail-panel:not([hidden])")?.textContent).toContain(
          "Manage Screen & System Audio Recording access in System Settings.",
        );
      });
      navigation.find((item) => item.textContent?.trim() === "Connection")!.click();
      await vi.waitFor(() => {
        expect(
          root.querySelector(".settings-detail-panel:not([hidden])")?.textContent,
        ).not.toContain("System Settings unavailable");
      });
    } finally {
      if (previousImplementation) mockedInvoke.mockImplementation(previousImplementation);
    }
  });

  it("updates nonselected Codex without selecting it and keeps pending progress targeted", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const mockedInvoke = vi.mocked(invoke);
    const previousImplementation = mockedInvoke.getMockImplementation();
    const previousSelection = snapshot.agent_selection;
    const previousConfig = snapshot.config;
    snapshot.agent_selection = {
      stage: "selected",
      candidate: "claude",
      supports_logout: false,
      auth_methods: [],
    };
    snapshot.config = { ...previousConfig, agent: "claude" };
    let finishUpdate!: (value: unknown) => void;
    mockedInvoke.mockImplementation(async (command, ...arguments_) => {
      if (command === "update_managed_agent")
        return await new Promise<unknown>((resolve) => {
          finishUpdate = resolve;
        });
      return previousImplementation?.(command, ...arguments_);
    });
    try {
      const page = await createPage("settings");
      const root = viewRoot(page, "lens-settings-view")!;
      await vi.waitFor(() =>
        expect(root.querySelector("lens-agent-settings")?.querySelector("button")).not.toBeNull(),
      );
      const editor = root.querySelector("lens-agent-settings")!;
      const codexUpdate = [...editor.querySelectorAll("button")].find(
        (item) => item.textContent?.trim() === "Install or Update Codex",
      )!;
      codexUpdate.click();
      await vi.waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("update_managed_agent", { agent: "codex" }),
      );
      expect(invoke).not.toHaveBeenCalledWith("set_agent", expect.anything());
      expect(snapshot.agent_selection.candidate).toBe("claude");
      await vi.waitFor(() =>
        expect(editor.querySelector(".runtime-status")?.textContent).toContain(
          "Checking Codex for updates…",
        ),
      );
      finishUpdate({
        agent: "codex",
        stage: "ready",
        downloaded_bytes: 0,
        message: "Installed Codex.",
      });
      await vi.waitFor(() =>
        expect(
          root.querySelector(".settings-context-feedback[role='status']")?.textContent,
        ).toContain("Installed Codex."),
      );
      expect(snapshot.agent_selection.candidate).toBe("claude");
    } finally {
      snapshot.agent_selection = previousSelection;
      snapshot.config = previousConfig;
      if (previousImplementation) mockedInvoke.mockImplementation(previousImplementation);
    }
  });

  it("keeps Goose file browsing local until Save and Verify", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const { open } = await import("@tauri-apps/plugin-dialog");
    const previousSelection = snapshot.agent_selection;
    const previousConfig = snapshot.config;
    snapshot.agent_selection = {
      stage: "failed",
      candidate: { external: "profile-1" },
      supports_logout: false,
      auth_methods: [],
      error: "Choose a Goose executable.",
    };
    snapshot.config = {
      ...previousConfig,
      agent: { external: "profile-1" },
      external_agents: [{ id: "profile-1", name: "Goose", command: "/saved/goose", args: ["acp"] }],
    };
    vi.mocked(open).mockResolvedValue("/chosen directory/goose");
    try {
      const page = await createPage("settings");
      const root = viewRoot(page, "lens-settings-view")!;
      const editor = root.querySelector("lens-agent-settings")!;
      const path = editor.querySelector<HTMLInputElement>('[aria-label="ACP command"]')!;
      path.value = "/draft/goose acp";
      path.dispatchEvent(new Event("input"));
      expect(invoke).not.toHaveBeenCalledWith("save_external_agent", expect.anything());
      [...editor.querySelectorAll("button")]
        .find((item) => item.textContent?.trim() === "Browse…")!
        .click();
      await vi.waitFor(() => expect(path.value).toBe("'/chosen directory/goose' acp"));
      expect(open).toHaveBeenCalledWith({
        directory: false,
        multiple: false,
        defaultPath: "/draft/goose",
        title: "Choose ACP Executable",
      });
      expect(invoke).not.toHaveBeenCalledWith("set_agent", expect.anything());
      expect(invoke).not.toHaveBeenCalledWith("save_external_agent", expect.anything());
      expect(snapshot.config.external_agents![0]!.command).toBe("/saved/goose");
      [...editor.querySelectorAll("button")]
        .find((item) => item.textContent?.trim() === "Save and Verify")!
        .click();
      await vi.waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("save_external_agent", {
          profile: {
            id: "profile-1",
            name: "Goose",
            command_line: "'/chosen directory/goose' acp",
          },
        }),
      );
    } finally {
      snapshot.agent_selection = previousSelection;
      snapshot.config = previousConfig;
    }
  });

  it("saves an advertised Agent mode without a separate privilege confirmation", async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const { DEFAULT_AGENT_DEFAULTS } = await import("./components/lens-agent-defaults");
    const previousSelection = snapshot.agent_selection;
    snapshot.agent_selection = {
      ...previousSelection,
      operation_id: operationId,
      agent_default: "default",
      modes: [
        { id: "default", name: "Default" },
        { id: "write", name: "Write" },
      ],
    };
    try {
      const page = await createPage("settings");
      const defaults = {
        ...structuredClone(DEFAULT_AGENT_DEFAULTS),
        choices: [{ config_id: "mode", value: "write" }],
      };
      page.querySelector("lens-settings-view")!.dispatchEvent(
        new CustomEvent("lens-settings-intent", {
          detail: { type: "save-agent-defaults", defaults },
          bubbles: true,
          composed: true,
        }),
      );
      await vi.waitFor(() =>
        expect(invoke).toHaveBeenCalledWith("set_agent_defaults", {
          selectionId: operationId,
          defaults,
        }),
      );
      expect(confirm).not.toHaveBeenCalled();
    } finally {
      snapshot.agent_selection = previousSelection;
    }
  });

  it("uses one System Settings-style navigation authority and one detail destination", async () => {
    const element = await createPage("settings");
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
    expect(items.map((item) => item.textContent?.trim())).toEqual([
      "Connection",
      "Session Defaults",
      "Prompt Presets",
      "Privacy & Security",
    ]);
    expect(
      Array.from(navigation?.querySelectorAll(".settings-nav-group") ?? []).map((group) => ({
        heading: group.querySelector(".settings-nav-heading")?.textContent?.trim(),
        destinations: Array.from(group.querySelectorAll("button")).map((button) =>
          button.textContent?.trim(),
        ),
      })),
    ).toEqual([
      { heading: "Agent", destinations: ["Connection", "Session Defaults", "Prompt Presets"] },
      { heading: "General", destinations: ["Privacy & Security"] },
    ]);
    expect(items[0]?.getAttribute("aria-current")).toBe("page");
    expect(
      settingsRoot?.querySelector(".settings-detail-panel:not([hidden]) h1")?.textContent,
    ).toBe("Connection");

    items.find((item) => item.textContent?.trim() === "Prompt Presets")?.click();
    await vi.waitFor(() => {
      expect(
        settingsRoot?.querySelector(".settings-detail-panel:not([hidden]) h1")?.textContent?.trim(),
      ).toBe("Prompt Presets");
    });
    expect(
      items
        .find((item) => item.textContent?.trim() === "Prompt Presets")
        ?.getAttribute("aria-current"),
    ).toBe("page");
    expect(settingsRoot?.querySelector(".prompt-preview summary")?.textContent?.trim()).toBe(
      "Prompt Preview",
    );
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
      const element = await createPage("settings");
      const settingsRoot = viewRoot(element, "lens-settings-view");
      await vi.waitFor(() => {
        expect(settingsRoot?.querySelector<LensSelect>("lens-select")).not.toBeNull();
      });

      const agentMenu = settingsRoot!.querySelector<LensSelect>("lens-select")!;
      await agentMenu.updateComplete;
      agentMenu.shadowRoot!.querySelector<HTMLButtonElement>("button")!.click();
      await agentMenu.updateComplete;
      agentMenu.shadowRoot!.querySelector<HTMLElement>('[data-index="0"]')!.click();

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
        .find((button) => button.textContent?.trim() === "Prompt Presets")
        ?.click();
      await vi.waitFor(() => {
        const visiblePanel = settingsRoot?.querySelector(".settings-detail-panel:not([hidden])");
        expect(visiblePanel?.querySelector("h1")?.textContent?.trim()).toBe("Prompt Presets");
        expect(visiblePanel?.querySelector(".settings-context-feedback[role='alert']")).toBeNull();
      });
    } finally {
      if (previousImplementation) mockedInvoke.mockImplementation(previousImplementation);
    }
  });

  it("preserves native controls and saves the complete prompt template atomically", async () => {
    const element = await createPage("settings");
    await vi.waitFor(() => {
      expect(
        viewRoot(element, "lens-settings-view")?.querySelector<HTMLInputElement>(".directory-field")
          ?.value,
      ).toBe("/tmp");
    });
    const settingsRoot = viewRoot(element, "lens-settings-view");
    const agentPrompt = Array.from(
      settingsRoot?.querySelectorAll<HTMLButtonElement>(".settings-nav-item") ?? [],
    ).find((button) => button.textContent?.trim() === "Prompt Presets");
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
      expect(invoke).toHaveBeenCalledWith("update_prompt_presets", {
        change: {
          type: "update",
          id: "conceptual",
          expected_revision: 1,
          name: "Conceptual",
          template: {
            ...snapshot.config.agent_prompt_template,
            common: "Updated instruction.\n\n{turn_instruction}",
          },
        },
      });
    });
    await vi.waitFor(() => {
      const feedback = settingsRoot?.querySelector(
        "lens-prompt-settings .settings-context-feedback[role='status']",
      );
      expect(feedback?.textContent).toContain("Prompt presets updated.");
    });
    expect(
      settingsRoot?.querySelector(".settings-sidebar-status [role='status']")?.textContent?.trim(),
    ).toBe("Transformation complete");
  });
});

it("correlates notification submissions, rejects duplicate clicks, and permits retry after failure", async () => {
  const { invoke } = await import("@tauri-apps/api/core");
  const element = await createPage("overlay");
  await vi.waitFor(() => {
    const view = element.querySelector("lens-overlay-view") as HTMLElement & {
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
      .querySelector("lens-overlay-view")!
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
    const view = element.querySelector("lens-overlay-view") as HTMLElement & {
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

it("recovery requires explicit prompt-only confirmation and sends the inspected file digest", async () => {
  const { invoke } = await import("@tauri-apps/api/core");
  const mocked = vi.mocked(invoke);
  const previous = mocked.getMockImplementation();
  mocked.mockImplementation(async (command, ...args) => {
    if (command === "get_settings_recovery")
      return {
        message: "Prompt presets are invalid",
        settings_path: "/fixture/settings.json",
        can_restore_prompt_presets: true,
        digest: "inspected-digest",
      };
    if (command === "restore_recovery_prompt_presets") return undefined;
    return previous?.(command, ...args);
  });
  try {
    const page = await createPage("settings-recovery");
    const root = viewRoot(page, "lens-settings-recovery-view")!;
    await vi.waitFor(() => expect(root.textContent).toContain("Restore Default Prompt Presets"));
    [...root.querySelectorAll("button")]
      .find((button) => button.textContent?.includes("Restore Default"))!
      .click();
    await vi.waitFor(() => expect(root.textContent).toContain("Other settings will be preserved"));
    expect(invoke).not.toHaveBeenCalledWith("restore_recovery_prompt_presets", expect.anything());
    [...root.querySelectorAll("button")]
      .find((button) => button.textContent?.trim() === "Restore Prompt Presets")!
      .click();
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("restore_recovery_prompt_presets", {
        expectedDigest: "inspected-digest",
      }),
    );
    expect(invoke).not.toHaveBeenCalledWith("get_app_snapshot");
  } finally {
    if (previous) mocked.mockImplementation(previous);
  }
});

it.each(["cancel", "success", "failure"] as const)(
  "Agent Presets reset handles %s without implicitly launching an agent",
  async (outcome) => {
    const { invoke } = await import("@tauri-apps/api/core");
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const original = vi.mocked(invoke).getMockImplementation()!;
    const previousConfig = snapshot.config;
    vi.mocked(confirm).mockResolvedValue(outcome !== "cancel");
    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "reset_external_agents") {
        if (outcome === "failure") throw new Error("Reset write failed");
        snapshot.config = {
          ...snapshot.config,
          external_agents: [
            {
              id: "preset-copilot",
              name: "GitHub Copilot",
              command: "copilot",
              args: ["--acp", "--stdio"],
            },
            { id: "preset-goose", name: "Goose", command: "goose", args: ["acp"] },
          ],
        };
        return snapshot.config;
      }
      return original(command);
    });
    try {
      const page = await createPage("settings");
      const root = viewRoot(page, "lens-settings-view")!;
      const editor = root.querySelector("lens-agent-settings")!;
      const click = (label: string) =>
        [...editor.querySelectorAll("button")]
          .find((item) => item.textContent?.trim() === label)!
          .click();
      click("Add preset");
      await vi.waitFor(() =>
        expect(editor.querySelector('[aria-label="ACP command"]')).not.toBeNull(),
      );
      const input = editor.querySelector<HTMLInputElement>('[aria-label="ACP command"]')!;
      input.value = "unsaved-agent --stdio";
      input.dispatchEvent(new Event("input"));
      click("Reset Agent Presets…");
      await vi.waitFor(() =>
        expect(confirm).toHaveBeenCalledWith(expect.stringContaining("unsaved drafts"), {
          title: "Reset Agent Presets",
          kind: "warning",
        }),
      );
      await vi.waitFor(() => {
        expect(
          vi.mocked(invoke).mock.calls.filter(([name]) => name === "reset_external_agents"),
        ).toHaveLength(outcome === "cancel" ? 0 : 1);
        expect(
          editor.querySelector<HTMLInputElement>('[aria-label="ACP command"]')?.value ?? null,
        ).toBe(outcome === "success" ? null : "unsaved-agent --stdio");
        expect(root.textContent?.includes("Reset write failed")).toBe(outcome === "failure");
        expect(
          [
            ...editor
              .querySelector<LensSelect>("lens-select")!
              .shadowRoot!.querySelectorAll('[role="option"] .lens-select-label'),
          ].map((option) => option.textContent?.trim()),
        ).toEqual(
          outcome === "success"
            ? ["Claude", "Codex", "GitHub Copilot", "Goose"]
            : ["Claude", "Codex", "New preset (unsaved)"],
        );
      });
      expect(invoke).not.toHaveBeenCalledWith("set_agent", expect.anything());
      expect(invoke).not.toHaveBeenCalledWith("save_external_agent", expect.anything());
    } finally {
      snapshot.config = previousConfig;
      vi.mocked(invoke).mockImplementation(original);
      vi.mocked(confirm).mockReset();
    }
  },
);
