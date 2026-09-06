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
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("progressive page attachment", () => {
  it("preserves the HTML header, native selection and focus made before the page module arrives", async () => {
    installDocument();
    const header = root().querySelector("header");
    const select = root().querySelector("select")!;
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
    select.focus();
    select.value = "notice";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    allowImport();
    await startup;
    await vi.waitFor(() =>
      expect(root().querySelector("lens-license-document")?.textContent).toBe("Notice"),
    );
    expect(root().querySelector("header")).toBe(header);
    expect(root().querySelector("select")).toBe(select);
    expect(root().activeElement).toBe(select);
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
