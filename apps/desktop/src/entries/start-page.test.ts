// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import aboutHtml from "../../about.html?raw";
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
  const parsed = new DOMParser().parseFromString(aboutHtml, "text/html");
  document.documentElement.dataset.view = "about";
  document.body.innerHTML = parsed.body.innerHTML;
  window.history.replaceState({}, "", "/about.html?platform=macos");
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("progressive page attachment", () => {
  it("preserves the HTML header, native selection and focus made before the page module arrives", async () => {
    installDocument();
    const header = document.querySelector("header");
    const select = document.querySelector("select")!;
    let allowImport!: () => void;
    const delayed = new Promise<void>((resolve) => {
      allowImport = resolve;
    });
    const startup = startPage("about", async () => {
      await delayed;
      return import("../pages/about-page");
    });
    expect(document.documentElement.dataset.platform).toBe("macos");
    expect(document.querySelector("h1")?.textContent).toBe("Lens");
    select.focus();
    select.value = "notice";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    allowImport();
    await startup;
    await vi.waitFor(() =>
      expect(document.querySelector("lens-license-document")?.textContent).toBe("Notice"),
    );
    expect(document.querySelector("header")).toBe(header);
    expect(document.querySelector("select")).toBe(select);
    expect(document.activeElement).toBe(select);
    expect(select.value).toBe("notice");
  });

  it("keeps available HTML and exposes recovery when its page module fails", async () => {
    installDocument();
    vi.spyOn(console, "error").mockImplementation(() => {});
    const header = document.querySelector("header");
    await startPage("about", async () => {
      throw new Error("Module unavailable");
    });
    expect(document.querySelector("header")).toBe(header);
    expect(document.querySelector<HTMLElement>("[data-page-error]")?.hidden).toBe(false);
    expect(document.querySelector("[data-page-error] button")?.textContent).toBe("Reload");
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
