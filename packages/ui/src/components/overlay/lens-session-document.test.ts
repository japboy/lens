import { mediaFixture } from "../../../tests/fixtures/media-parity";
import { ResponseHistoryController } from "../../resources/response-history-controller";
// @vitest-environment jsdom
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  LensSessionDocument,
  LensConversationBlock,
  conversationRows,
} from "./lens-session-document";
import { ConversationRenderCache } from "../../resources/conversation-render-cache";
import { LensOverlayView } from "../views/lens-overlay-view";
import type {
  DeferredDocumentBlock,
  DocumentBlock,
  SessionDocument,
} from "../../contracts/session-document";

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
function deferredBody() {
  let resolve!: (block: DocumentBlock) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<DocumentBlock>((resolveBody, rejectBody) => {
    resolve = resolveBody;
    reject = rejectBody;
  });
  return { promise, resolve, reject };
}
function conversationCell() {
  const cell = new LensConversationBlock();
  cell.scopeId = "session-generation";
  cell.cellId = "assistant:0";
  cell.cache = new ConversationRenderCache();
  return cell;
}
function reviseCell(
  cell: LensConversationBlock,
  revision: number,
  content_type: DeferredDocumentBlock["content_type"] = "markdown",
) {
  cell.block = {
    type: "deferred",
    entry_id: "assistant",
    block_index: 0,
    content_type,
    revision,
    byte_length: revision,
    append_only: content_type === "markdown",
  };
  cell.contentKey = `assistant:0:${revision}`;
}
function displayedText(cell: LensConversationBlock) {
  return cell.shadowRoot!.querySelector<import("./lens-conversation-text").LensConversationText>(
    "lens-conversation-text",
  );
}
describe("Conversation body ownership", () => {
  it("uses the normal empty host with aria-busy before the first body arrives", async () => {
    const body = deferredBody();
    const cell = conversationCell();
    reviseCell(cell, 1);
    cell.loadBlock = () => body.promise;
    document.body.append(cell);
    await cell.updateComplete;
    expect(cell.getAttribute("aria-busy")).toBe("true");
    expect(cell.shadowRoot!.querySelector("[role=status], .loading")).toBeNull();
    expect(cell.shadowRoot!.textContent).not.toContain("Loading");
    body.resolve({ type: "markdown", text: "First body" });
    await vi.waitFor(() => {
      expect(displayedText(cell)?.text).toBe("First body");
      expect(cell.getAttribute("aria-busy")).toBe("false");
    });
  });

  it("retains displayed text and its DOM while fetching a newer revision of the same cell", async () => {
    const newer = deferredBody();
    const cell = conversationCell();
    reviseCell(cell, 1);
    const load = vi.fn<NonNullable<LensConversationBlock["loadBlock"]>>(
      async (reference): Promise<DocumentBlock> =>
        reference.revision === 1 ? { type: "markdown", text: "Existing body" } : newer.promise,
    );
    cell.loadBlock = load;
    document.body.append(cell);
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("Existing body"));
    const text = displayedText(cell);
    reviseCell(cell, 2);
    await cell.updateComplete;
    expect(displayedText(cell)).toBe(text);
    expect(text?.text).toBe("Existing body");
    expect(cell.getAttribute("aria-busy")).toBe("true");
    expect(load).toHaveBeenCalledTimes(2);
    newer.resolve({ type: "markdown", text: "Existing body plus an update" });
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("Existing body plus an update"));
    expect(displayedText(cell)).toBe(text);
  });

  it("does not restart an in-flight request when the same descriptor version is recreated", async () => {
    const body = deferredBody();
    const cell = conversationCell();
    reviseCell(cell, 1);
    const load = vi.fn<NonNullable<LensConversationBlock["loadBlock"]>>(() => body.promise);
    cell.loadBlock = load;
    document.body.append(cell);
    await cell.updateComplete;
    reviseCell(cell, 1);
    await cell.updateComplete;
    expect(load).toHaveBeenCalledTimes(1);
    body.resolve({ type: "markdown", text: "Original request completes" });
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("Original request completes"));
    expect(cell.shadowRoot!.querySelector("[role=alert]")).toBeNull();
  });

  it.each(["scope", "cell", "kind"] as const)(
    "clears old content on %s reassignment, even if the cache contentKey is unchanged",
    async (boundary) => {
      const next = deferredBody();
      const cell = conversationCell();
      reviseCell(cell, 1);
      const load = vi
        .fn<NonNullable<LensConversationBlock["loadBlock"]>>()
        .mockResolvedValueOnce({ type: "markdown", text: "Other identity's body" })
        .mockReturnValueOnce(next.promise);
      cell.loadBlock = load;
      document.body.append(cell);
      await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("Other identity's body"));
      if (boundary === "scope") cell.scopeId = "new-session-generation";
      else if (boundary === "cell") cell.cellId = "different-entry:0";
      else reviseCell(cell, 1, "html");
      await cell.updateComplete;
      expect(displayedText(cell)).toBeNull();
      expect(cell.getAttribute("aria-busy")).toBe("true");
      next.resolve({ type: boundary === "kind" ? "html" : "markdown", text: "New identity body" });
      await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("New identity body"));
      expect(load).toHaveBeenCalledTimes(2);
    },
  );

  it.each(["resolve", "reject"] as const)(
    "ignores a superseded revision's late %s while the newest request owns the cell",
    async (completion) => {
      const old = deferredBody();
      const current = deferredBody();
      const cell = conversationCell();
      reviseCell(cell, 1);
      cell.loadBlock = (reference) => (reference.revision === 1 ? old.promise : current.promise);
      document.body.append(cell);
      await cell.updateComplete;
      reviseCell(cell, 2);
      await cell.updateComplete;
      if (completion === "resolve") old.resolve({ type: "markdown", text: "Stale body" });
      else old.reject(new Error("Stale failure"));
      current.resolve({ type: "markdown", text: "Current body" });
      await vi.waitFor(() => {
        expect(displayedText(cell)?.text).toBe("Current body");
        expect(cell.getAttribute("aria-busy")).toBe("false");
      });
      expect(cell.shadowRoot!.querySelector("[role=alert]")).toBeNull();
      expect(cell.shadowRoot!.textContent).not.toContain("Stale");
    },
  );

  it("retains the prior body on update failure and retries only the failed current request", async () => {
    const retry = deferredBody();
    const cell = conversationCell();
    reviseCell(cell, 1);
    const load = vi
      .fn<NonNullable<LensConversationBlock["loadBlock"]>>()
      .mockResolvedValueOnce({ type: "markdown", text: "Retained body" })
      .mockRejectedValueOnce(new Error("Current request failed"))
      .mockReturnValueOnce(retry.promise);
    cell.loadBlock = load;
    document.body.append(cell);
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("Retained body"));
    const text = displayedText(cell);
    reviseCell(cell, 2);
    await vi.waitFor(() => {
      expect(cell.shadowRoot!.querySelector("[role=alert]")?.textContent).toContain(
        "Current request failed",
      );
      expect(cell.getAttribute("aria-busy")).toBe("false");
    });
    expect(displayedText(cell)).toBe(text);
    reviseCell(cell, 2);
    await cell.updateComplete;
    expect(load).toHaveBeenCalledTimes(2);
    cell.remove();
    document.body.append(cell);
    await cell.updateComplete;
    expect(load).toHaveBeenCalledTimes(2);
    expect(cell.shadowRoot!.querySelector("[role=alert]")?.textContent).toContain(
      "Current request failed",
    );
    const button = cell.shadowRoot!.querySelector<HTMLButtonElement>("button")!;
    button.click();
    button.click();
    await cell.updateComplete;
    expect(load).toHaveBeenCalledTimes(3);
    expect(cell.getAttribute("aria-busy")).toBe("true");
    expect(displayedText(cell)).toBe(text);
    expect(cell.shadowRoot!.querySelector("[role=alert]")).toBeNull();
    retry.resolve({ type: "markdown", text: "Recovered body" });
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("Recovered body"));
    expect(displayedText(cell)).toBe(text);
  });

  it("retains only the displayed version when the bounded cache cannot retain its body", async () => {
    const first = deferredBody();
    const second = deferredBody();
    const cell = conversationCell();
    cell.cache = new ConversationRenderCache(1);
    reviseCell(cell, 1);
    cell.loadBlock = (reference) => (reference.revision === 1 ? first.promise : second.promise);
    document.body.append(cell);
    await cell.updateComplete;
    first.resolve({ type: "markdown", text: "Body larger than the entire cache budget" });
    await vi.waitFor(() =>
      expect(displayedText(cell)?.text).toBe("Body larger than the entire cache budget"),
    );
    expect(cell.cache.retainedEntries).toBe(0);
    const text = displayedText(cell);
    reviseCell(cell, 2);
    await cell.updateComplete;
    expect(displayedText(cell)).toBe(text);
    second.resolve({ type: "markdown", text: "New larger body" });
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("New larger body"));
    expect(cell.cache.retainedBytes).toBe(0);
  });

  it("waits until reconnect to fetch a revision received while the cell is detached", async () => {
    const cell = conversationCell();
    reviseCell(cell, 1);
    const load = vi.fn<NonNullable<LensConversationBlock["loadBlock"]>>(
      async (reference): Promise<DocumentBlock> => ({
        type: "markdown",
        text: `Revision ${reference.revision}`,
      }),
    );
    cell.loadBlock = load;
    document.body.append(cell);
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("Revision 1"));
    cell.remove();
    reviseCell(cell, 2);
    await cell.updateComplete;
    expect(load).toHaveBeenCalledTimes(1);
    document.body.append(cell);
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe("Revision 2"));
    expect(load).toHaveBeenCalledTimes(2);
    expect(cell.getAttribute("aria-busy")).toBe("false");
  });
});
describe("canonical session renderer", () => {
  it("displays resource-link diagnostic metadata literally without opening or loading its URI", async () => {
    const metadata =
      "Name: <img src=x onerror=alert(1)>\nTitle: [App](https://example.com)\nDescription: <script>never execute</script>\nURI: ui://lens/publication/2";
    const cell = conversationCell();
    cell.block = { type: "markdown", text: metadata };
    cell.contentKey = "resource-metadata";
    document.body.append(cell);
    await vi.waitFor(() => expect(displayedText(cell)?.text).toBe(metadata));
    const text = displayedText(cell)!;
    await text.updateComplete;
    expect(text.shadowRoot!.querySelector(".text")!.textContent).toBe(metadata);
    expect(cell.shadowRoot!.querySelector("iframe, img, a, script")).toBeNull();
    expect(text.shadowRoot!.querySelector("iframe, img, a, script")).toBeNull();
  });
  it("keeps ordered messages and displays HTML literally without loading embedded resources", async () => {
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
    const text =
      block.shadowRoot!.querySelector<import("./lens-conversation-text").LensConversationText>(
        "lens-conversation-text",
      )!;
    await text.updateComplete;
    expect(text.shadowRoot!.querySelector(".text")!.textContent).toBe(
      documentModel.entries[1]!.blocks[0]!.type === "html"
        ? documentModel.entries[1]!.blocks[0]!.text
        : "",
    );
    expect(block.shadowRoot!.querySelector("iframe, img, a, script")).toBeNull();
    expect(text.shadowRoot!.querySelector("iframe, img, a, script")).toBeNull();
  });
  it("shows image metadata without exposing or decoding its payload", async () => {
    const block = new LensConversationBlock();
    block.block = { type: "image", mime_type: "image/png", data: "private-base64-payload" };
    block.contentKey = "image-metadata";
    block.cache = new ConversationRenderCache();
    document.body.append(block);
    await block.updateComplete;
    await new Promise((resolve) => setTimeout(resolve, 0));
    await block.updateComplete;
    expect(block.shadowRoot!.textContent).toContain("Image: image/png");
    expect(block.shadowRoot!.textContent).not.toContain("private-base64-payload");
    expect(block.shadowRoot!.querySelector("img, iframe, canvas")).toBeNull();
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
  it("shows a session-specific empty answer for a ready replay with no response manifests", async () => {
    const view = new LensOverlayView();
    view.active = true;
    view.sessionView = {
      revision: 1,
      phase: "ready",
      generation: "empty",
      interpretation: { responses: [] },
      document: { entries: [] },
    };
    const fixture = mediaFixture("none");
    const controller = new ResponseHistoryController(view, fixture.port, fixture.loadSessionBlock);
    controller.synchronizeHistory(view.sessionView);
    view.responseHistory = controller.presentation;
    document.body.append(view);
    await view.updateComplete;
    const output =
      view.shadowRoot!.querySelector<import("./lens-agent-output").LensAgentOutput>(
        "lens-agent-output",
      )!;
    await output.updateComplete;
    expect(output.textContent).toContain("No answer is available in this session.");
    expect(output.textContent).not.toContain("Lens Targets");
    expect(output.querySelector(".lens-response")).toBeNull();
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
    const fixture = mediaFixture("single");
    view.sessionView = {
      ...view.sessionView!,
      generation: "saved-generation",
      interpretation: fixture.interpretation,
    };
    const controller = new ResponseHistoryController(view, fixture.port, fixture.loadSessionBlock);
    controller.synchronizeHistory(view.sessionView);
    view.responseHistory = controller.presentation;
    view.loadResponseBlock = controller.loadBlock;
    document.body.append(view);
    await view.updateComplete;
    const output =
      view.shadowRoot!.querySelector<import("./lens-agent-output").LensAgentOutput>(
        "lens-agent-output",
      )!;
    await output.updateComplete;
    expect(output.history?.scopeId).toBe("history:saved-generation");
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

it("history keyboard navigation visits only Interpretation and Conversation", async () => {
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

it("restored generation changes reset the history media reading position", async () => {
  const { LensAgentOutput } = await import("./lens-agent-output");
  const view = new LensAgentOutput();
  const fixture = mediaFixture("single");
  const controller = new ResponseHistoryController(view, fixture.port, fixture.loadSessionBlock);
  const session = {
    revision: 1,
    phase: "ready" as const,
    generation: "first",
    interpretation: fixture.interpretation,
  };
  controller.synchronizeHistory(session);
  view.history = controller.presentation;
  view.loadResponseBlock = controller.loadBlock;
  document.body.append(view);
  await view.updateComplete;
  view.querySelector<HTMLElement>(".lens-output")!.scrollTop = 250;
  controller.synchronizeHistory({ ...session, generation: "second" });
  view.history = controller.presentation;
  await view.updateComplete;
  await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  expect(view.querySelector<HTMLElement>(".lens-output")!.scrollTop).toBe(0);
});
