import type { LensSelect } from "../components/lens-select";
// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { installGeneratedPage } from "../rendering/generated-page.test-helper";
import type { WebviewPort } from "../application/webview-port";
import { startPage } from "./start-page";

vi.mock("../application/webview-port", () => ({
  tauriWebviewPort: {
    getAboutInfo: vi.fn<WebviewPort["getAboutInfo"]>(async () => ({
      name: "Lens",
      version: "1.2.3",
      copyright: "Fixture",
    })),
    getAboutDocuments: vi.fn<WebviewPort["getAboutDocuments"]>(async () => ({
      license: "License",
      notice: "Notice",
    })),
  },
}));

function installDocument(): void {
  installGeneratedPage("about");
}
function root(): ShadowRoot {
  return document.querySelector("lens-about-view")!.shadowRoot!;
}

afterEach(() => {
  window.dispatchEvent(new Event("pagehide"));
  delete window.__LENS_CONTROL_PALETTE__;
  document.documentElement.style.removeProperty("--control-background");
  delete document.documentElement.dataset.increaseContrast;
  delete document.documentElement.dataset.reduceTransparency;
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("progressive page attachment", () => {
  it("colors ready Settings DSD before its page code loads without replacing the readonly field", async () => {
    installGeneratedPage("settings");
    const view = document.querySelector("lens-settings-view")!;
    const field = view.shadowRoot!.querySelector<HTMLInputElement>(".directory-field")!;
    window.__LENS_CONTROL_PALETTE__ = {
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
      increase_contrast: true,
      reduce_transparency: false,
    };
    let release!: () => void;
    const delayed = new Promise<void>((resolve) => {
      release = resolve;
    });
    vi.spyOn(console, "error").mockImplementation(() => {});
    const startup = startPage("settings", async () => {
      await delayed;
      throw new Error("Test stops before page attachment");
    });
    expect(document.documentElement.style.getPropertyValue("--control-background")).toBe(
      "rgb(248 248 248)",
    );
    expect(view.getAttribute("data-increase-contrast")).toBe("true");
    expect(field.getAttribute("data-lens-control")).toBe("text-entry");
    expect(field.closest("[data-settings-surface]")).toBeNull();
    expect(field.readOnly).toBe(true);
    expect(field.disabled).toBe(false);
    field.focus();
    field.value = "/fixture/path";
    field.setSelectionRange(0, 8);
    window.dispatchEvent(new CustomEvent("lens-control-palette", { detail: null }));
    expect(view.shadowRoot!.querySelector(".directory-field")).toBe(field);
    expect(view.shadowRoot!.activeElement).toBe(field);
    expect(field.selectionEnd).toBe(8);
    expect(document.documentElement.style.getPropertyValue("--control-background")).toBe("");
    release();
    await startup;
  });

  it("preserves the HTML header, document selection and focus made before the page module arrives", async () => {
    installDocument();
    const header = root().querySelector("header");
    const select = root().querySelector<LensSelect>("lens-select")!;
    let allowImport!: () => void;
    const delayed = new Promise<void>((resolve) => {
      allowImport = resolve;
    });
    const startup = startPage("about", async () => {
      await delayed;
      return import("../pages/about-page");
    });
    expect(document.documentElement.dataset.platform).toBe("macos");
    expect(root().querySelector("h1")?.textContent).toBe("Lens");
    select.shadowRoot!.querySelector<HTMLButtonElement>("button")!.focus();
    select.value = "notice";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    allowImport();
    await startup;
    await vi.waitFor(() =>
      expect(root().querySelector("lens-license-document")?.textContent).toBe("Notice"),
    );
    expect(root().querySelector("header")).toBe(header);
    expect(root().querySelector<LensSelect>("lens-select")).toBe(select);
    expect(root().activeElement).toBe(select);
    expect(select.shadowRoot!.activeElement).toBe(select.shadowRoot!.querySelector("button"));
    expect(select.value).toBe("notice");
  });

  it("keeps available HTML and exposes recovery when its page module fails", async () => {
    installDocument();
    vi.spyOn(console, "error").mockImplementation(() => {});
    const header = root().querySelector("header");
    await startPage("about", async () => {
      throw new Error("Module unavailable");
    });
    expect(root().querySelector("header")).toBe(header);
    expect(document.querySelector<HTMLElement>("[data-page-error]")?.hidden).toBe(false);
    expect(document.querySelector("[data-page-error] button")?.textContent).toBe("Reload");
  });

  it("rejects an empty client-created shadow root instead of silently falling back to CSR", async () => {
    installDocument();
    await startPage("about", () => import("../pages/about-page"));
    const page = document.createElement("lens-about-page");
    const view = document.createElement("lens-about-view");
    view.setAttribute("defer-hydration", "");
    view.attachShadow({ mode: "open" });
    page.append(view);
    const recovery = document.querySelector("[data-page-error]")!;
    document.body.replaceChildren(page, recovery);
    vi.spyOn(console, "error").mockImplementation(() => {});
    await startPage("about", () => import("../pages/about-page"));
    expect(document.querySelector<HTMLElement>("[data-page-error]")?.hidden).toBe(false);
    expect(view.shadowRoot?.querySelector("main")).toBeNull();
  });

  it("rejects mismatched entry identity before loading a different page's code", async () => {
    installDocument();
    vi.spyOn(console, "error").mockImplementation(() => {});
    const load = vi.fn<() => Promise<unknown>>();
    await startPage("settings", load);
    expect(load).not.toHaveBeenCalled();
    expect(document.querySelector<HTMLElement>("[data-page-error]")?.hidden).toBe(false);
  });
});
