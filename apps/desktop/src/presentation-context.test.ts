// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import {
  applyPresentationContext,
  APP_VIEWS,
  DESKTOP_PLATFORMS,
  presentationContextForPage,
} from "./presentation-context";

describe("presentation context", () => {
  it("accepts every finite view and desktop platform pair", () => {
    for (const view of APP_VIEWS) {
      for (const platform of DESKTOP_PLATFORMS) {
        expect(presentationContextForPage(view, view, `?platform=${platform}`)).toEqual({
          view,
          platform,
        });
      }
    }
  });

  it.each([
    ["wrong document", "overlay", "?platform=macos", "Page identity mismatch"],
    ["missing document", undefined, "?platform=macos", "Page identity mismatch"],
    ["query view", "settings", "?view=settings&platform=macos", "Query-based view routing"],
    ["missing platform", "settings", "", "Invalid platform"],
    ["unknown platform", "settings", "?platform=ios", "Invalid platform"],
  ])("rejects %s", (_name, documentView, search, expectedMessage) => {
    expect(() => presentationContextForPage("settings", documentView, search!)).toThrow(
      expectedMessage,
    );
  });

  it("publishes the explicit context to every presentation boundary", () => {
    const elements = [document.createElement("html"), document.createElement("body")];

    applyPresentationContext({ view: "settings", platform: "macos" }, elements);

    for (const element of elements) {
      expect(element.dataset.view).toBe("settings");
      expect(element.dataset.platform).toBe("macos");
    }
  });
});
