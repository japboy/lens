// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import {
  applyPresentationContext,
  APP_VIEWS,
  DESKTOP_PLATFORMS,
  presentationContextFromSearch,
} from "./presentation-context";

describe("presentation context", () => {
  it("accepts every finite view and desktop platform pair", () => {
    for (const view of APP_VIEWS) {
      for (const platform of DESKTOP_PLATFORMS) {
        expect(presentationContextFromSearch(`?view=${view}&platform=${platform}`)).toEqual({
          view,
          platform,
        });
      }
    }
  });

  it.each([
    ["missing view", "?platform=macos", "Invalid view presentation state"],
    ["unknown view", "?view=main&platform=macos", "Invalid view presentation state"],
    ["missing platform", "?view=settings", "Invalid platform presentation state"],
    ["unknown platform", "?view=settings&platform=ios", "Invalid platform presentation state"],
  ])("rejects %s", (_name, search, expectedMessage) => {
    expect(() => presentationContextFromSearch(search)).toThrow(expectedMessage);
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
