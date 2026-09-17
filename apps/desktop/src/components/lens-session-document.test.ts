// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { LensSessionDocument } from "./lens-session-document";
import { LensOverlayView } from "./lens-overlay-view";
import type { SessionDocument } from "../application/session-document";

const documentModel: SessionDocument = {
  entries: [
    { id: "u", kind: "message", role: "user", blocks: [{ type: "markdown", text: "Question" }] },
    {
      id: "t",
      kind: "tool",
      title: "Render",
      status: "completed",
      blocks: [
        {
          type: "html",
          text: '<h1>Saved</h1><script>alert(1)</script><img src="https://example.com/track">',
        },
      ],
    },
    {
      id: "a",
      kind: "message",
      role: "assistant",
      blocks: [{ type: "image", mime_type: "image/png", data: "aA==" }],
    },
  ],
};
beforeAll(async () => {
  await import("./lens-agent-output");
  window.matchMedia ??= () =>
    ({
      matches: false,
      addEventListener: () => {},
      removeEventListener: () => {},
    }) as unknown as MediaQueryList;
});
afterEach(() => document.body.replaceChildren());
describe("canonical session renderer", () => {
  it("keeps ordered messages and tool output and applies the existing HTML sandbox policy", async () => {
    const view = new LensSessionDocument();
    view.document = documentModel;
    document.body.append(view);
    await view.updateComplete;
    const root = view.shadowRoot!;
    expect(
      [...root.querySelectorAll("article")].map((e) => e.getAttribute("data-entry-id")),
    ).toEqual(["u", "t", "a"]);
    const frame = root.querySelector("iframe")!;
    expect(frame.getAttribute("sandbox")).toBe("allow-popups");
    expect(frame.srcdoc).toContain("Saved");
    expect(frame.srcdoc).not.toContain("<script>");
    expect(frame.srcdoc).toContain("img-src data:");
    expect(frame.srcdoc).toContain("connect-src 'none'");
    expect(root.querySelector("img")?.src).toBe("data:image/png;base64,aA==");
  });
  it("renders history without live controls or needing a live snapshot", async () => {
    const view = new LensOverlayView();
    view.active = true;
    view.sessionView = {
      revision: 1,
      phase: "ready",
      agent: "codex",
      session_id: "saved",
      document: documentModel,
    };
    document.body.append(view);
    await view.updateComplete;
    const output =
      view.shadowRoot!.querySelector<import("./lens-agent-output").LensAgentOutput>(
        "lens-agent-output",
      )!;
    await output.updateComplete;
    expect(output.presentation?.artifactIdentity).toBe("codex:saved");
    expect(output.querySelector("lens-output-media")).not.toBeNull();
    expect(view.shadowRoot!.querySelector(".overlay-shell .overlay-brand")).not.toBeNull();
    expect(view.shadowRoot!.querySelector(".overlay-footer")).not.toBeNull();
    expect(view.shadowRoot!.querySelector("#source-tab")).toBeNull();
    expect(view.shadowRoot!.querySelector("lens-session-controls")).toBeNull();
    expect(view.shadowRoot!.querySelector('[aria-label="Close Lens"]')).not.toBeNull();
    expect(view.shadowRoot!.textContent).not.toContain("Resume");
  });
  it("uses the same canonical document renderer for live Conversation and restored content", async () => {
    const live = new LensOverlayView();
    live.active = true;
    live.sessionView = {
      revision: 1,
      phase: "live",
      agent: "codex",
      session_id: "same",
      document: documentModel,
    };
    const history = new LensOverlayView();
    history.active = true;
    history.sessionView = { ...live.sessionView, phase: "ready" };
    document.body.append(live, history);
    await Promise.all([live.updateComplete, history.updateComplete]);
    live.shadowRoot!.querySelector<HTMLButtonElement>("#conversation-tab")!.click();
    history.shadowRoot!.querySelector<HTMLButtonElement>("#conversation-tab")!.click();
    await Promise.all([live.updateComplete, history.updateComplete]);
    const liveDocument =
      live.shadowRoot!.querySelector<LensSessionDocument>("lens-session-document")!;
    const historyDocument =
      history.shadowRoot!.querySelector<LensSessionDocument>("lens-session-document")!;
    await Promise.all([liveDocument.updateComplete, historyDocument.updateComplete]);
    expect(liveDocument.document).toBe(historyDocument.document);
    expect(liveDocument.shadowRoot!.innerHTML).toBe(historyDocument.shadowRoot!.innerHTML);
  });
  it("shows a live document capacity failure alongside the retained content", async () => {
    const view = new LensOverlayView();
    view.active = true;
    view.sessionView = {
      revision: 2,
      phase: "live",
      document: documentModel,
      error: "Session document capacity exceeded",
    };
    document.body.append(view);
    await view.updateComplete;
    view.shadowRoot!.querySelector<HTMLButtonElement>("#conversation-tab")!.click();
    await view.updateComplete;
    expect(view.shadowRoot!.querySelector('#conversation-panel [role="alert"]')?.textContent).toBe(
      "Session document capacity exceeded",
    );
    const rendered = view.shadowRoot!.querySelector<LensSessionDocument>("lens-session-document")!;
    await rendered.updateComplete;
    expect(rendered.document).toBe(documentModel);
    expect(rendered.shadowRoot!.querySelectorAll("article")).toHaveLength(3);
  });
});

it("history keyboard navigation only visits the two available tabs", async () => {
  const view = new LensOverlayView();
  view.active = true;
  view.sessionView = {
    revision: 1,
    phase: "ready",
    agent: "codex",
    session_id: "saved",
    document: documentModel,
  };
  document.body.append(view);
  await view.updateComplete;
  for (const [key, expected] of [
    ["ArrowRight", "conversation"],
    ["ArrowRight", "interpretation"],
    ["End", "conversation"],
    ["Home", "interpretation"],
    ["ArrowLeft", "conversation"],
  ]) {
    view
      .shadowRoot!.querySelector<HTMLButtonElement>('[aria-selected="true"]')!
      .dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
    await view.updateComplete;
    expect(view.shadowRoot!.querySelector('[aria-selected="true"]')?.id).toBe(`${expected}-tab`);
    expect(view.shadowRoot!.querySelector(`#${expected}-panel`)).not.toBeNull();
  }
});

it("restored presentation identity changes reveal the first media", async () => {
  const { LensAgentOutput } = await import("./lens-agent-output");
  const view = new LensAgentOutput();
  view.presentation = {
    identity: "first",
    artifactIdentity: "first",
    mode: "settled",
    blocks: [{ type: "image", mime_type: "image/png", data: "aA==" }],
  };
  document.body.append(view);
  await view.updateComplete;
  view.querySelector<HTMLElement>(".lens-output")!.scrollTop = 250;
  view.presentation = { ...view.presentation, identity: "second", artifactIdentity: "second" };
  await view.updateComplete;
  expect(view.querySelector<HTMLElement>(".lens-output")!.scrollTop).toBe(0);
});
