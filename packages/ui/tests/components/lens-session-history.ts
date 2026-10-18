// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { LensSessionHistory } from "../../src/components/settings/lens-session-history";
import type { HistoryEntry } from "../../src/contracts/view-models";

afterEach(() => document.body.replaceChildren());

const entry: HistoryEntry = {
  id: "opaque/source/session",
  title: "Saved session",
  agent: "ChatGPT Codex",
  cwd: "/workspace",
  updated_at: "2026-10-18T03:20:00Z",
  state: "listed",
  available: true,
};

it("presents local readable dates while preserving the wire timestamp in semantic time", async () => {
  const view = new LensSessionHistory();
  view.history = {
    stage: "ready",
    operation: "idle",
    offset: 0,
    page: {
      revision: 1,
      total: 3,
      entries: [
        entry,
        { ...entry, id: "undated", updated_at: null },
        { ...entry, id: "invalid", updated_at: "invalid timestamp" },
      ],
    },
  };
  document.body.append(view);
  await view.updateComplete;
  const time = view.shadowRoot!.querySelector("time")!;
  expect(time.getAttribute("datetime")).toBe(entry.updated_at);
  expect(time.textContent).toBe(
    new Intl.DateTimeFormat(undefined, { dateStyle: "short", timeStyle: "short" }).format(
      new Date(entry.updated_at!),
    ),
  );
  expect(time.textContent).not.toBe(entry.updated_at);
  expect(view.shadowRoot!.querySelectorAll("time")).toHaveLength(1);
  expect(view.shadowRoot!.textContent!.match(/Unknown date/g)).toHaveLength(2);
});

it("keeps opaque IDs in intents and renders untrusted titles as text", async () => {
  const view = new LensSessionHistory();
  view.history = {
    stage: "ready",
    operation: "idle",
    offset: 0,
    page: { revision: 1, total: 1, entries: [{ ...entry, title: "<img src=x onerror=alert(1)>" }] },
  };
  let intent: unknown;
  view.addEventListener("lens-history-intent", (event) => {
    intent = (event as CustomEvent).detail;
  });
  document.body.append(view);
  await view.updateComplete;
  expect(view.shadowRoot!.querySelector("img")).toBeNull();
  expect(view.shadowRoot!.querySelector(".agent-icon")?.getAttribute("aria-hidden")).toBe("true");
  expect(view.shadowRoot!.querySelector(".agent-name")?.textContent).toBe(entry.agent);
  expect(view.shadowRoot!.textContent).toContain("<img src=x onerror=alert(1)>");
  const open = Array.from(view.shadowRoot!.querySelectorAll("button")).find(
    (button) => button.textContent?.trim() === "Open",
  )!;
  open.click();
  expect(intent).toEqual({ type: "open-history", id: entry.id });
  view.history = { ...view.history, operation: "confirming" };
  await view.updateComplete;
  expect(
    Array.from(view.shadowRoot!.querySelectorAll("button")).every((button) => button.disabled),
  ).toBe(true);
});

it("reports incomplete sync targets and offers list recovery only after errors", async () => {
  const view = new LensSessionHistory();
  view.history = {
    stage: "ready",
    operation: "idle",
    offset: 0,
    page: { revision: 1, total: 0, entries: [] },
    outcome: {
      kind: "sync",
      result: { succeeded: 2, failed: 1, purged: 3, errors: ["Partial listing"] },
    },
  };
  document.body.append(view);
  await view.updateComplete;
  const result = view.shadowRoot!.querySelector("[role='status']")!;
  expect(result.textContent).toContain("2 Agent/workspace combinations refreshed");
  expect(result.textContent).toContain("1 failed or incomplete");
  expect(result.textContent).toContain("3 removed from Lens");
  expect(result.textContent).not.toContain("connections failed");
  expect(view.shadowRoot!.textContent).not.toContain("Refresh List");
  view.history = {
    ...view.history,
    stage: "failed",
    error: "Deletion failed",
    readError: "Stale catalog",
  };
  await view.updateComplete;
  expect(view.shadowRoot!.textContent).toContain("Refresh List");
  expect(view.shadowRoot!.textContent).toContain("Deletion failed");
  expect(view.shadowRoot!.textContent).toContain("Stale catalog");
});
