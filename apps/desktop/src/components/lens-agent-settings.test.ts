// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { LensAgentSettings } from "./lens-agent-settings";
import type { AgentIntent } from "./events";

afterEach(() => document.body.replaceChildren());
it("shows the history-selected agent without authenticated controls and permits verification", async () => {
  const element = new LensAgentSettings();
  element.selection = { stage: "history_selected", candidate: "claude", auth_methods: [] };
  element.runtime = { stage: "ready", agent: "claude", downloaded_bytes: 0 };
  const listener = vi.fn<(event: Event) => void>();
  element.addEventListener("lens-agent-intent", listener);
  document.body.append(element);
  await element.updateComplete;
  expect(element.querySelector<HTMLInputElement>('input[value="claude"]')?.checked).toBe(true);
  expect(element.querySelector(".status-ok")).toBeNull();
  expect(element.textContent).not.toContain("Reauthenticate");
  const verify = [...element.querySelectorAll("button")].find((button) =>
    button.textContent?.includes("Verify connection"),
  )!;
  expect(verify.disabled).toBe(false);
  verify.click();
  expect(listener).toHaveBeenCalledOnce();
  expect((listener.mock.calls[0]![0] as CustomEvent<AgentIntent>).detail).toEqual({
    type: "select",
    agent: "claude",
  });
});
