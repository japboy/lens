// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  LensSessionDocument,
  LensConversationBlock,
  conversationRows,
} from "./lens-session-document";
import { ConversationRenderCache } from "../application/conversation-render-cache";
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
    expect(conversationRows(documentModel).map((row) => row.id)).toEqual([
      "u:header",
      "u:0",
      "t:header",
      "t:0",
      "a:header",
      "a:0",
    ]);
    const block = new LensConversationBlock();
    block.block = documentModel.entries[1]!.blocks[0];
    block.contentKey = "sandbox";
    block.cache = new ConversationRenderCache();
    document.body.append(block);
    await block.updateComplete;
    await new Promise((resolve) => setTimeout(resolve, 20));
    await block.updateComplete;
    const frame = block.shadowRoot!.querySelector("iframe")!;
    expect(frame.getAttribute("sandbox")).toBe("allow-popups");
    expect(frame.srcdoc).toContain("Saved");
    expect(frame.srcdoc).not.toContain("<script>");
    expect(frame.srcdoc).toContain("img-src data:");
    expect(frame.srcdoc).toContain("connect-src 'none'");
  });
  it("builds rows for a large manifest without requesting any body", () => {
    const model: SessionDocument = {
      entries: Array.from({ length: 10000 }, (_, i) => ({
        id: String(i),
        kind: "message",
        role: "assistant",
        blocks: [
          {
            type: "deferred",
            entry_id: String(i),
            block_index: 0,
            content_type: "markdown",
            revision: 1,
            byte_length: 1000000,
          },
        ],
      })),
    };
    expect(conversationRows(model)).toHaveLength(20000);
  });
  it("retries interrupted visible preparation after reconnect", async () => {
    const block = new LensConversationBlock();
    const cache = new ConversationRenderCache();
    block.cache = cache;
    block.contentKey = "reconnect";
    block.block = {
      type: "deferred",
      entry_id: "a",
      block_index: 0,
      content_type: "unsupported",
      revision: 1,
      byte_length: 1,
    };
    let finish!: (value: { type: "unsupported"; content_type: string }) => void;
    block.loadBlock = () =>
      new Promise((resolve) => {
        finish = resolve;
      });
    document.body.append(block);
    await block.updateComplete;
    block.remove();
    cache.suspend();
    finish({ type: "unsupported", content_type: "stale" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    block.loadBlock = async () => ({ type: "unsupported", content_type: "fresh" });
    document.body.append(block);
    await block.updateComplete;
    await new Promise((resolve) => setTimeout(resolve, 0));
    await block.updateComplete;
    expect(block.shadowRoot!.textContent).toContain("fresh");
    expect(block.shadowRoot!.textContent).not.toContain("stale");
  });
  it("restores a saved row offset after a cached tab reconnects", async () => {
    const view = new LensSessionDocument();
    view.identity = "anchor-contract";
    view.document = documentModel;
    document.body.append(view);
    await view.updateComplete;
    const virtualizer = view.shadowRoot!.querySelector("lit-virtualizer")!;
    await virtualizer.updateComplete;
    Object.defineProperties(virtualizer, {
      clientHeight: { configurable: true, value: 100 },
      scrollHeight: { configurable: true, value: 1000 },
      layoutComplete: { configurable: true, get: () => Promise.resolve() },
    });
    virtualizer.scrollTop = 120;
    const row = document.createElement("div");
    row.setAttribute("data-row-id", "u:header");
    row.getBoundingClientRect = () => ({ top: -12 }) as DOMRect;
    virtualizer.append(row);
    virtualizer.dispatchEvent(new Event("scroll"));
    const scrollIntoView = vi.fn<(options?: ScrollIntoViewOptions) => void>(() => {
      virtualizer.scrollTop = 120;
    });
    Object.defineProperty(virtualizer, "element", {
      configurable: true,
      value: () => ({ scrollIntoView }),
    });
    view.remove();
    virtualizer.scrollTop = 0;
    document.body.append(view);
    await view.updateComplete;
    await Promise.resolve();
    expect(scrollIntoView).toHaveBeenCalledWith({ block: "start" });
    expect(virtualizer.scrollTop).toBe(132);
  });
  it.each(["wheel", "touchstart", "keyboard", "programmatic"])(
    "respects %s while an asynchronous anchor restoration is pending",
    async (input) => {
      const view = new LensSessionDocument();
      view.identity = `pending-anchor-${input}`;
      view.document = documentModel;
      document.body.append(view);
      await view.updateComplete;
      const virtualizer = view.shadowRoot!.querySelector("lit-virtualizer")!;
      await virtualizer.updateComplete;
      let complete!: () => void;
      const layout = new Promise<void>((resolve) => {
        complete = resolve;
      });
      Object.defineProperties(virtualizer, {
        clientHeight: { configurable: true, value: 100 },
        scrollHeight: { configurable: true, value: 1000 },
        layoutComplete: { configurable: true, get: () => layout },
        element: {
          configurable: true,
          value: () => ({
            scrollIntoView: () => {
              virtualizer.scrollTop = 120;
            },
          }),
        },
      });
      virtualizer.scrollTop = 120;
      const row = document.createElement("div");
      row.setAttribute("data-row-id", "u:header");
      row.getBoundingClientRect = () => ({ top: -12 }) as DOMRect;
      virtualizer.append(row);
      virtualizer.dispatchEvent(new Event("scroll"));
      view.remove();
      document.body.append(view);
      await view.updateComplete;
      if (input === "keyboard")
        virtualizer.dispatchEvent(new KeyboardEvent("keydown", { key: "PageDown", bubbles: true }));
      else if (input !== "programmatic")
        virtualizer.dispatchEvent(new Event(input, { bubbles: true }));
      virtualizer.scrollTop = 240;
      virtualizer.dispatchEvent(new Event("scroll"));
      complete();
      await layout;
      await Promise.resolve();
      expect(virtualizer.scrollTop).toBe(input === "programmatic" ? 252 : 240);
    },
  );
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
    expect(conversationRows(rendered.document)).toHaveLength(6);
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
