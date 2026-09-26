// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { controlStyles } from "./component-styles";

describe("app-owned text entry selector", () => {
  it("covers owned text inputs, number inputs and textarea, excluding labels and rendered output", () => {
    const selector = controlStyles.cssText
      .slice(
        controlStyles.cssText.indexOf('[data-lens-control="text-entry"]'),
        controlStyles.cssText.indexOf(
          "{",
          controlStyles.cssText.indexOf('[data-lens-control="text-entry"]'),
        ),
      )
      .trim();
    for (const type of ["text", "search", "url", "tel", "email", "password", "number"]) {
      const input = document.createElement("input");
      input.type = type;
      expect(input.matches(selector)).toBe(false);
      input.dataset.lensControl = "text-entry";
      expect(input.matches(selector)).toBe(true);
    }
    for (const tag of ["input", "textarea"]) {
      const control = document.createElement(tag);
      control.dataset.lensControl = "text-entry";
      expect(control.matches(selector)).toBe(true);
    }
    for (const type of ["checkbox", "radio", "button", "range", "color", "file", "hidden"]) {
      const input = document.createElement("input");
      input.type = type;
      input.dataset.lensControl = "text-entry";
      expect(input.matches(selector)).toBe(false);
    }
    const label = document.createElement("label");
    label.dataset.lensControl = "text-entry";
    expect(label.matches(selector)).toBe(false);
    expect(document.createElement("textarea").matches(selector)).toBe(false);
  });
});
