// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { APP_VIEWS, DESKTOP_PLATFORMS } from "./presentation-context";
import { installControlPalette } from "./control-palette";

const context = { view: "settings", platform: "macos" } as const;
const light = {
  colors: {
    control_surface: [248, 248, 248, 255],
    window_surface: [255, 255, 255, 255],
    button_fill: [0, 0, 0, 20],
    button_pressed_fill: [0, 0, 0, 25],
    primary_button_fill: [0, 114, 240, 255],
    primary_button_foreground: [255, 255, 255, 255],
    separator: [0, 0, 0, 25],
  },
  window_active: true,
  increase_contrast: false,
  reduce_transparency: false,
};
const dark = {
  colors: {
    control_surface: [36, 36, 36, 255],
    window_surface: [30, 30, 30, 255],
    button_fill: [255, 255, 255, 20],
    button_pressed_fill: [255, 255, 255, 25],
    primary_button_fill: [255, 220, 0, 255],
    primary_button_foreground: [0, 0, 0, 255],
    separator: [255, 255, 255, 25],
  },
  window_active: false,
  increase_contrast: true,
  reduce_transparency: true,
};
let root: HTMLElement;
let view: HTMLElement;
function mount(seed: unknown = light): void {
  root = document.createElement("div");
  view = document.createElement("lens-settings-view");
  window.__LENS_CONTROL_PALETTE__ = seed;
  installControlPalette(context, root, view);
}
function publish(detail: unknown): void {
  window.dispatchEvent(new CustomEvent("lens-control-palette", { detail }));
}
afterEach(() => {
  window.dispatchEvent(new Event("pagehide"));
  delete window.__LENS_CONTROL_PALETTE__;
  vi.restoreAllMocks();
});

describe("native control palette", () => {
  it("installs the seeded colors synchronously and replaces the complete appearance snapshot", () => {
    mount();
    expect(root.style.getPropertyValue("--control-background")).toBe("rgb(248 248 248)");
    expect(view.dataset.increaseContrast).toBe("false");
    publish(dark);
    expect(root.style.getPropertyValue("--control-background")).toBe("rgb(36 36 36)");
    expect(view.dataset.increaseContrast).toBe("true");
    expect(root.dataset.reduceTransparency).toBe("true");
    publish(light);
    expect(view.dataset.increaseContrast).toBe("false");
    expect(root.dataset.reduceTransparency).toBe("false");
  });

  it.each([
    null,
    undefined,
    {},
    { ...light, colors: { ...light.colors, control_surface: "red" } },
    { ...light, colors: { ...light.colors, control_surface: [1, 2, 3] } },
    { ...light, colors: { ...light.colors, control_surface: Object.assign([], { 3: 255 }) } },
    { ...light, colors: { ...light.colors, control_surface: [1, 2, 3, 255, 9] } },
    { ...light, colors: { ...light.colors, control_surface: [1, -1, 3, 255] } },
    { ...light, colors: { ...light.colors, control_surface: [1, 256, 3, 255] } },
    { ...light, colors: { ...light.colors, control_surface: [1, 2.5, 3, 255] } },
    { ...light, colors: { ...light.colors, control_surface: [1, NaN, 3, 255] } },
    { ...light, colors: { ...light.colors, control_surface: [1, 2, 3, 128] } },
    { ...light, colors: undefined },
    { ...light, colors: { ...light.colors, window_surface: [1, 2, 3, 128] } },
    { ...light, colors: { ...light.colors, button_fill: [1, 2, 3, 256] } },
    { ...light, colors: { ...light.colors, button_pressed_fill: [1, 2, NaN, 20] } },
    { ...light, colors: { ...light.colors, primary_button_fill: [0, 114, 240, 254] } },
    { ...light, colors: { ...light.colors, primary_button_foreground: [255, 255, 255, 128] } },
    { ...light, colors: { ...light.colors, primary_button_fill: undefined } },
    { ...light, colors: { ...light.colors, primary_button_foreground: [0, 0, 0] } },
    { ...light, colors: { ...light.colors, separator: "red" } },
    { ...light, window_active: null },
    { ...light, increase_contrast: "true" },
    { ...light, reduce_transparency: null },
  ])("falls back atomically for unavailable or invalid data %#", (invalid) => {
    mount(dark);
    publish(invalid);
    expect(root.style.getPropertyValue("--control-background")).toBe("");
    expect(view.hasAttribute("data-increase-contrast")).toBe(false);
    expect(root.hasAttribute("data-reduce-transparency")).toBe(false);
    publish(light);
    expect(root.style.getPropertyValue("--control-background")).toBe("rgb(248 248 248)");
  });

  it("replaces the opaque primary pair together and clears both when either member is invalid", () => {
    mount();
    expect(root.style.getPropertyValue("--native-primary-button-fill")).toBe("rgb(0 114 240)");
    expect(root.style.getPropertyValue("--native-primary-button-foreground")).toBe(
      "rgb(255 255 255)",
    );
    publish(dark);
    expect(root.style.getPropertyValue("--native-primary-button-fill")).toBe("rgb(255 220 0)");
    expect(root.style.getPropertyValue("--native-primary-button-foreground")).toBe("rgb(0 0 0)");
    publish({ ...light, colors: { ...light.colors, primary_button_foreground: null } });
    expect(root.style.getPropertyValue("--native-primary-button-fill")).toBe("");
    expect(root.style.getPropertyValue("--native-primary-button-foreground")).toBe("");
    expect(root.dataset.nativeControls).toBeUndefined();
  });

  it("preserves raw alpha and replaces window activation independently of colors", () => {
    mount();
    expect(root.style.getPropertyValue("--native-button-fill")).toBe(`rgb(0 0 0 / ${20 / 255})`);
    expect(root.style.getPropertyValue("--window-background")).toBe("rgb(255 255 255)");
    expect(root.dataset.nativeControls).toBe("true");
    expect(view.dataset.windowActive).toBe("true");
    publish({ ...light, window_active: false });
    expect(view.dataset.windowActive).toBe("false");
    expect(root.style.getPropertyValue("--native-button-fill")).toBe(`rgb(0 0 0 / ${20 / 255})`);
  });

  it("removes every native color on resolution failure while retaining accessibility and window state", () => {
    mount();
    publish({ ...dark, colors: null });
    expect(root.style.length).toBe(0);
    expect(root.dataset.nativeControls).toBe("false");
    expect(root.dataset.increaseContrast).toBe("true");
    expect(root.dataset.reduceTransparency).toBe("true");
    expect(view.dataset.windowActive).toBe("false");
    publish(light);
    expect(root.dataset.nativeControls).toBe("true");
    expect(root.style.getPropertyValue("--window-background")).toBe("rgb(255 255 255)");
  });

  it.each(APP_VIEWS)("applies the same palette in the macOS %s view", (appView) => {
    window.__LENS_CONTROL_PALETTE__ = light;
    const boundary = document.createElement("div");
    const viewBoundary = document.createElement(`lens-${appView}-view`);
    installControlPalette({ view: appView, platform: "macos" }, boundary, viewBoundary);
    expect(boundary.style.getPropertyValue("--control-background")).toBe("rgb(248 248 248)");
    publish(dark);
    expect(boundary.style.getPropertyValue("--control-background")).toBe("rgb(36 36 36)");
    expect(viewBoundary.dataset.increaseContrast).toBe("true");
  });

  it("does not apply or subscribe on non-macOS platforms", () => {
    window.__LENS_CONTROL_PALETTE__ = light;
    const listen = vi.spyOn(window, "addEventListener");
    for (const appView of APP_VIEWS) {
      for (const platform of DESKTOP_PLATFORMS) {
        if (platform === "macos") continue;
        const boundary = document.createElement("div");
        installControlPalette({ view: appView, platform }, boundary, null);
        publish(dark);
        expect(boundary.style.length).toBe(0);
        expect(boundary.hasAttribute("data-increase-contrast")).toBe(false);
      }
    }
    expect(listen.mock.calls.some(([name]) => name === "lens-control-palette")).toBe(false);
  });

  it("detaches the previous view on reattachment and releases its listener at pagehide", () => {
    mount();
    const previousView = view;
    view = document.createElement("lens-settings-view");
    installControlPalette(context, root, view);
    publish(dark);
    expect(previousView.dataset.increaseContrast).toBe("false");
    expect(view.dataset.increaseContrast).toBe("true");
    window.dispatchEvent(new Event("pagehide"));
    publish(light);
    expect(root.style.getPropertyValue("--control-background")).toBe("rgb(36 36 36)");
  });

  it("retains its subscription while the same document is stored in the back-forward cache", () => {
    mount();
    window.dispatchEvent(new PageTransitionEvent("pagehide", { persisted: true }));
    window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true }));
    publish(dark);
    expect(root.style.getPropertyValue("--control-background")).toBe("rgb(36 36 36)");
    window.dispatchEvent(new PageTransitionEvent("pagehide", { persisted: false }));
    publish(light);
    expect(root.style.getPropertyValue("--control-background")).toBe("rgb(36 36 36)");
  });

  it("restores semantic CSS fallback on reattachment without a native seed", () => {
    mount();
    delete window.__LENS_CONTROL_PALETTE__;
    installControlPalette(context, root, view);
    expect(root.style.length).toBe(0);
    expect(view.hasAttribute("data-increase-contrast")).toBe(false);
  });
});
