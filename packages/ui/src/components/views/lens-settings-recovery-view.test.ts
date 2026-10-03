// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { LensSettingsRecoveryView } from "./lens-settings-recovery-view";
afterEach(() => document.body.replaceChildren());
it("offers prompt-only restore only with verified eligibility and digest", async () => {
  const element = new LensSettingsRecoveryView();
  element.info = {
    message: "Invalid settings",
    settings_path: "/settings.json",
    can_restore_prompt_presets: false,
    digest: null,
  };
  document.body.append(element);
  await element.updateComplete;
  expect(element.textContent).toBe("");
  expect(element.shadowRoot!.textContent).toContain("has not been replaced");
  expect(element.shadowRoot!.textContent).not.toContain("Restore Default");
  element.info = { ...element.info, can_restore_prompt_presets: true, digest: "digest" };
  await element.updateComplete;
  expect(element.shadowRoot!.textContent).toContain("Restore Default Prompt Presets");
  element.confirming = true;
  await element.updateComplete;
  expect(element.shadowRoot!.textContent).toContain("Other settings will be preserved");
  const actions: string[] = [];
  element.addEventListener("settings-recovery-action", (event) =>
    actions.push((event as CustomEvent<string>).detail),
  );
  [...element.shadowRoot!.querySelectorAll("button")]
    .find((button) => button.textContent === "Cancel")!
    .click();
  expect(actions).toEqual(["cancel"]);
});
