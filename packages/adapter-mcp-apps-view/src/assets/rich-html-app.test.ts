import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { JSDOM } from "jsdom";
import { randomUUID } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
});

function shell(capabilities: Record<string, unknown>) {
  let receive!: (event: { source: unknown; origin: string; data: unknown }) => void;
  const parent = { postMessage: vi.fn<Window["postMessage"]>() };
  const frame = {
    contentWindow: { postMessage: vi.fn<Window["postMessage"]>() },
    setAttribute: vi.fn<Element["setAttribute"]>(),
    addEventListener: vi.fn<EventTarget["addEventListener"]>(),
    remove: vi.fn<Element["remove"]>(),
    src: "",
  };
  const append = vi.fn<(frame: unknown) => void>();
  const source = readFileSync(new URL("./rich-html-app.html", import.meta.url), "utf8");
  const script = source.match(/<script>([\s\S]*?)<\/script>/)![1]!;
  const dom = new JSDOM(source.replace(/<script>[\s\S]*?<\/script>/, ""));
  const document = dom.window.document;
  vi.spyOn(document.getElementById("document")!, "append").mockImplementation(append);
  const create = document.createElement.bind(document);
  vi.spyOn(document, "createElement").mockImplementation(((tag: string) =>
    tag === "iframe" ? frame : create(tag)) as typeof document.createElement);
  runInNewContext(script, {
    parent,
    URL,
    TextEncoder,
    window: {
      addEventListener: (_: string, handler: typeof receive) => {
        receive = handler;
      },
    },
    document,
    crypto: { randomUUID },
    setTimeout,
    clearTimeout,
  });
  const host = (data: unknown) =>
    receive({ source: parent, origin: "http://127.0.0.1:43162", data });
  const child = (data: unknown) => receive({ source: frame.contentWindow, origin: "null", data });
  host({
    jsonrpc: "2.0",
    id: "lens-initialize",
    result: {
      hostCapabilities: capabilities,
      hostContext: { lensDocumentUrl: "http://127.0.0.1:43162/document/lease" },
    },
  });
  host({
    jsonrpc: "2.0",
    method: "ui/notifications/tool-result",
    params: {
      content: [],
      structuredContent: { html: "<!doctype html><script>unchanged</script>" },
    },
  });
  return { parent, frame, append, host, child, receive, document, dom };
}

describe("bundled rich HTML App shell", () => {
  it("loads the original natural document URL and forwards only source-checked negotiated requests", () => {
    const test = shell({ message: { text: {} }, updateModelContext: { text: {} } });
    expect(test.frame.src).toBe("http://127.0.0.1:43162/document/lease");
    expect(test.frame.setAttribute).toHaveBeenCalledWith("sandbox", "allow-scripts allow-forms");
    expect(test.frame).not.toHaveProperty("srcdoc");
    const params = { role: "user", content: [{ type: "text", text: "Please explain 73" }] };
    test.child({ jsonrpc: "2.0", id: "child-1", method: "ui/message", params });
    expect(test.parent.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "lens-document-1", method: "ui/message", params },
      "*",
    );
    test.host({ jsonrpc: "2.0", id: "lens-document-1", result: {} });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "child-1", result: {} },
      "*",
    );
    const count = test.parent.postMessage.mock.calls.length;
    test.child({ jsonrpc: "2.0", id: 4, method: "tools/call", params: { name: "unavailable" } });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(count);
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({
        id: 4,
        error: { code: -32601, message: "App operation unavailable" },
      }),
      "*",
    );
  });
  it("forwards standard open-link for a read-only App only when its capability is negotiated", () => {
    const test = shell({ openLinks: {} });
    const params = { url: "https://example.com/73" };
    test.child({ jsonrpc: "2.0", id: "anchor-1", method: "ui/open-link", params });
    expect(test.parent.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "lens-document-1", method: "ui/open-link", params },
      "*",
    );
    test.host({ jsonrpc: "2.0", id: "lens-document-1", result: { isError: false } });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "anchor-1", result: { isError: false } },
      "*",
    );
    test.child({ jsonrpc: "2.0", id: "context", method: "ui/update-model-context", params: {} });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({
        id: "context",
        error: { code: -32601, message: "App operation unavailable" },
      }),
      "*",
    );
  });
  it("rejects foreign frames, nonopaque children and unmatched responses while repeated tool results retain the document", () => {
    const test = shell({ openLinks: {} });
    const request = {
      jsonrpc: "2.0",
      id: "same-child-id",
      method: "ui/open-link",
      params: { url: "https://example.com" },
    };
    const before = test.parent.postMessage.mock.calls.length;
    test.receive({ source: {}, origin: "null", data: request });
    test.receive({
      source: test.frame.contentWindow,
      origin: "https://example.com",
      data: request,
    });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(before);
    test.child(request);
    test.child(request);
    expect(
      test.parent.postMessage.mock.calls
        .slice(-2)
        .map(([message]) => (message as { id: string }).id),
    ).toEqual(["lens-document-1", "lens-document-2"]);
    const replies = test.frame.contentWindow.postMessage.mock.calls.length;
    test.receive({
      source: {},
      origin: "http://127.0.0.1:43162",
      data: { jsonrpc: "2.0", id: "lens-document-1", result: { isError: false } },
    });
    test.host({ jsonrpc: "2.0", id: "unmatched", result: {} });
    expect(test.frame.contentWindow.postMessage).toHaveBeenCalledTimes(replies);
    test.host({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: {} });
    const afterRemount = test.frame.contentWindow.postMessage.mock.calls.length;
    test.host({ jsonrpc: "2.0", id: "lens-document-1", result: { isError: false } });
    test.host({ jsonrpc: "2.0", id: "lens-document-2", result: { isError: false } });
    expect(test.frame.contentWindow.postMessage).toHaveBeenCalledTimes(afterRemount + 2);
    expect(test.append).toHaveBeenCalledOnce();
    test.child(request);
    test.host({ jsonrpc: "2.0", id: "lens-document-3", result: { isError: false } });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "same-child-id", result: { isError: false } },
      "*",
    );
    test.host({ jsonrpc: "2.0", id: "teardown", method: "ui/resource-teardown", params: {} });
    const closedReplies = test.frame.contentWindow.postMessage.mock.calls.length;
    test.host({ jsonrpc: "2.0", id: "lens-document-3", result: { isError: false } });
    expect(test.frame.contentWindow.postMessage).toHaveBeenCalledTimes(closedReplies);
  });
  it("keeps retained historical content read-only and revokes pending messages on teardown", () => {
    const test = shell({});
    const count = test.parent.postMessage.mock.calls.length;
    test.child({ jsonrpc: "2.0", id: 2, method: "ui/message", params: {} });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(count);
    test.host({ jsonrpc: "2.0", id: "teardown", method: "ui/resource-teardown", params: {} });
    expect(test.frame.remove).toHaveBeenCalledOnce();
    expect(test.parent.postMessage).toHaveBeenLastCalledWith(
      { jsonrpc: "2.0", id: "teardown", result: {} },
      "*",
    );
    const after = test.parent.postMessage.mock.calls.length;
    test.child({ jsonrpc: "2.0", id: 3, method: "ui/message", params: {} });
    test.host({ jsonrpc: "2.0", method: "ui/notifications/tool-result", params: {} });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(after);
    expect(test.append).toHaveBeenCalledOnce();
  });
});

describe("generated App question round trip", () => {
  function toolReply(test: ReturnType<typeof shell>, name: string, result: unknown) {
    const request = test.parent.postMessage.mock.calls
      .map(
        ([value]) =>
          value as {
            id: string;
            method?: string;
            params?: { name?: string; arguments?: Record<string, unknown> };
          },
      )
      .reverse()
      .find((value) => value.method === "tools/call" && value.params?.name === name)!;
    expect(request).toBeDefined();
    test.host({ jsonrpc: "2.0", id: request.id, result });
    return request;
  }
  function lastSnapshot(test: ReturnType<typeof shell>) {
    const call = test.frame.contentWindow.postMessage.mock.calls
      .map(
        ([value]) =>
          value as {
            method?: string;
            params: { state: string; selection: string; answers: unknown };
          },
      )
      .reverse()
      .find((value) => value.method === "lens/questions/changed")!;
    return call.params;
  }
  const questionId = "b40e9e32-1515-4a30-a7a5-4a872695fc46";
  const completed = {
    appId: "app",
    revision: 2,
    latestApp: { artifactId: "updated", revision: 2 },
    questions: [
      { questionId, question: "Explain this", status: "completed", answer: "An explanation." },
    ],
  };
  function generatedDocument(test: ReturnType<typeof shell>) {
    const dom = new JSDOM(
      '<form><label>Explain this diagram<textarea></textarea></label><button>Explain</button></form><aside aria-live="polite"></aside>',
      { runScripts: "outside-only" },
    );
    const parent = { postMessage: (data: unknown) => test.child(data) };
    Object.defineProperty(dom.window, "parent", { value: parent });
    Object.assign(dom.window, { structuredClone });
    test.frame.contentWindow.postMessage.mockImplementation((data: unknown) => {
      dom.window.dispatchEvent(
        new dom.window.MessageEvent("message", { source: parent as unknown as Window, data }),
      );
    });
    dom.window.eval(readFileSync(new URL("./rich-html-links.js", import.meta.url), "utf8"));
    dom.window.eval(`
      const api = window.lensQuestions;
      api.subscribe(snapshot => {
        document.querySelector('button').disabled = snapshot.state !== 'idle';
        document.querySelector('aside').textContent = (snapshot.answers?.questions ?? []).map(item => item.answer ?? item.status).join('\\n');
      });
      document.querySelector('form').addEventListener('submit', async event => {
        event.preventDefault();
        await api.ask({question:document.querySelector('textarea').value});
        document.querySelector('textarea').value = '';
      });
    `);
    return dom;
  }
  it("executes a context-specific generated form, receives native answers and restores them after remount without changing the historical document", async () => {
    const test = shell({ serverTools: {} });
    expect(test.document.querySelector("form")).toBeNull();
    const child = generatedDocument(test);
    toolReply(test, "get_answers", {
      structuredContent: { appId: "app", revision: 0, questions: [] },
    });
    expect(child.window.document.querySelector("button")!.disabled).toBe(false);
    test.child({
      jsonrpc: "2.0",
      id: "selection",
      method: "ui/update-model-context",
      params: { structuredContent: { lensSelection: "Selected diagram" } },
    });
    const input = child.window.document.querySelector("textarea")!;
    input.value = "Explain this";
    child.window.document.querySelector("form")!.dispatchEvent(new child.window.Event("submit"));
    expect(child.window.document.querySelector("button")!.disabled).toBe(true);
    const ask = toolReply(test, "ask_question", {
      structuredContent: { questionId, status: "pending" },
    });
    expect(ask.params!.arguments).toMatchObject({
      question: "Explain this",
      selection: "Selected diagram",
    });
    expect(ask.params!.arguments!.questionId).toMatch(/^[a-f0-9-]{36}$/);
    toolReply(test, "get_answers", {
      structuredContent: completed,
      _meta: { lensDocumentUrl: "http://127.0.0.1:43162/document/lease/updated" },
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(child.window.document.querySelector("aside")!.textContent).toBe("An explanation.");
    expect(input.value).toBe("");
    expect(test.frame.src).toBe("http://127.0.0.1:43162/document/lease");
    expect(test.append).toHaveBeenCalledOnce();
    child.window.close();
    const reopened = shell({ serverTools: {} });
    const restored = generatedDocument(reopened);
    toolReply(reopened, "get_answers", { structuredContent: completed });
    expect(restored.window.document.querySelector("aside")!.textContent).toBe("An explanation.");
    expect(reopened.frame.src).toBe("http://127.0.0.1:43162/document/lease");
    restored.window.close();
  });
  it("exposes immutable API, isolates snapshot mutations, clears selection and rejects expired admission", async () => {
    const test = shell({ serverTools: {} });
    const child = generatedDocument(test);
    const api = child.window as unknown as {
      lensQuestions: {
        version: number;
        getSnapshot(): Promise<{
          state: string;
          selection: string;
          answers: { questions: unknown[] };
        }>;
        clearSelection(): Promise<{ selection: string }>;
        ask(value: { question: string }): Promise<unknown>;
      };
    };
    expect(Object.isFrozen(api.lensQuestions)).toBe(true);
    expect(api.lensQuestions.version).toBe(1);
    toolReply(test, "get_answers", { structuredContent: completed });
    const snapshot = await api.lensQuestions.getSnapshot();
    snapshot.answers.questions.length = 0;
    expect((await api.lensQuestions.getSnapshot()).answers.questions).toHaveLength(1);
    test.child({
      jsonrpc: "2.0",
      id: "selection",
      method: "ui/update-model-context",
      params: { structuredContent: { lensSelection: "A figure" } },
    });
    expect((await api.lensQuestions.getSnapshot()).selection).toBe("A figure");
    expect((await api.lensQuestions.clearSelection()).selection).toBe("");
    const denied = api.lensQuestions.ask({ question: "Try again" });
    toolReply(test, "ask_question", { isError: true });
    await expect(denied).rejects.toThrow("Question was not accepted");
    child.window.close();
    const expired = shell({ serverTools: {} });
    const expiredDocument = generatedDocument(expired);
    vi.advanceTimersByTime(30000);
    const expiredApi = expiredDocument.window as unknown as typeof api;
    await expect(expiredApi.lensQuestions.ask({ question: "Too late" })).rejects.toThrow(
      "timed out",
    );
    expect((await expiredApi.lensQuestions.getSnapshot()).state).toBe("expired");
    expiredDocument.window.close();
  });
  it("rejects invalid arguments, busy requests and unnegotiated tools without granting new authority", () => {
    const test = shell({ serverTools: {} });
    const before = test.parent.postMessage.mock.calls.length;
    test.child({
      jsonrpc: "2.0",
      id: "early",
      method: "lens/questions/ask",
      params: { question: "Too early" },
    });
    expect(test.parent.postMessage).toHaveBeenCalledTimes(before);
    toolReply(test, "get_answers", {
      structuredContent: { appId: "app", revision: 0, questions: [] },
    });
    test.child({
      jsonrpc: "2.0",
      id: "invalid",
      method: "lens/questions/ask",
      params: { question: "x".repeat(8001) },
    });
    expect(test.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({
        id: "invalid",
        error: { code: -32602, message: "Invalid question arguments" },
      }),
      "*",
    );
    test.child({
      jsonrpc: "2.0",
      id: "follow",
      method: "lens/questions/ask",
      params: { question: "And why?", parentQuestionId: questionId, selection: "" },
    });
    const follow = toolReply(test, "ask_question", { structuredContent: { status: "pending" } });
    expect(follow.params!.arguments).toMatchObject({
      parentQuestionId: questionId,
      question: "And why?",
    });
    expect(follow.params!.arguments).not.toHaveProperty("selection");
    const readonly = shell({});
    readonly.child({ jsonrpc: "2.0", id: "read", method: "lens/questions/snapshot" });
    expect(readonly.frame.contentWindow.postMessage).toHaveBeenLastCalledWith(
      expect.objectContaining({
        id: "read",
        result: expect.objectContaining({ state: "readonly" }),
      }),
      "*",
    );
    const count = readonly.parent.postMessage.mock.calls.length;
    readonly.child({
      jsonrpc: "2.0",
      id: "ask",
      method: "lens/questions/ask",
      params: { question: "Denied" },
    });
    expect(readonly.parent.postMessage).toHaveBeenCalledTimes(count);
  });
  it("preserves the last complete snapshot while polling unchanged pending revisions and stops at terminal state", () => {
    const test = shell({ serverTools: {} });
    toolReply(test, "get_answers", {
      structuredContent: {
        appId: "app",
        revision: 1,
        questions: [{ questionId, question: "Pending", status: "pending" }],
      },
    });
    vi.advanceTimersByTime(1000);
    const unchanged = toolReply(test, "get_answers", {
      structuredContent: { appId: "app", revision: 1, questions: [] },
    });
    expect(unchanged.params!.arguments).toEqual({ afterRevision: 1 });
    expect(lastSnapshot(test).state).toBe("pending");
    expect(lastSnapshot(test).answers).toMatchObject({ questions: [{ status: "pending" }] });
    vi.advanceTimersByTime(1000);
    toolReply(test, "get_answers", { structuredContent: completed });
    const count = test.parent.postMessage.mock.calls.length;
    vi.advanceTimersByTime(60000);
    expect(test.parent.postMessage).toHaveBeenCalledTimes(count);
    expect(lastSnapshot(test).state).toBe("idle");
  });
  it("expires timed out fetches, ignores late delivery and revokes callbacks on teardown", () => {
    const test = shell({ serverTools: {} });
    vi.advanceTimersByTime(30000);
    expect(lastSnapshot(test).state).toBe("expired");
    toolReply(test, "get_answers", { structuredContent: completed });
    expect(lastSnapshot(test).answers).toBeNull();
    const closed = shell({ serverTools: {} });
    closed.host({ jsonrpc: "2.0", id: "teardown", method: "ui/resource-teardown", params: {} });
    const count = closed.frame.contentWindow.postMessage.mock.calls.length;
    toolReply(closed, "get_answers", { structuredContent: completed });
    expect(closed.frame.contentWindow.postMessage).toHaveBeenCalledTimes(count);
  });
});
