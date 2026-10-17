// Versioned presentation API only. Durable Q&A and tool authority stay in Lens.
(() => {
  let sequence = 0;
  let current = { version: 1, state: "loading", selection: "", message: "", answers: null };
  const pending = new Map();
  const listeners = new Set();
  function update(value) {
    if (value?.version !== 1 || typeof value.state !== "string") return;
    current = value;
    for (const listener of listeners) {
      try {
        listener(structuredClone(current));
      } catch {
        /* A renderer cannot block other subscribers. */
      }
    }
  }
  function request(method, params = {}) {
    if (pending.size >= 32) return Promise.reject(new Error("Too many question requests."));
    const id = `lens-questions-client-${++sequence}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error("Question request timed out."));
      }, 31000);
      pending.set(id, { resolve, reject, timer, method });
      try {
        parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*");
      } catch (error) {
        pending.delete(id);
        clearTimeout(timer);
        reject(error);
      }
    });
  }
  window.addEventListener("message", (event) => {
    if (event.source !== parent || event.data?.jsonrpc !== "2.0") return;
    const message = event.data;
    if (message.method === "lens/questions/changed") {
      update(message.params);
      return;
    }
    const entry = pending.get(message.id);
    if (!entry || message.method !== undefined) return;
    pending.delete(message.id);
    clearTimeout(entry.timer);
    if (message.error) entry.reject(new Error(message.error.message || "Question request failed."));
    else {
      if (entry.method !== "lens/questions/ask") update(message.result);
      entry.resolve(message.result);
    }
  });
  Object.defineProperty(window, "lensQuestions", {
    value: Object.freeze({
      version: 1,
      ask: (question) => request("lens/questions/ask", question),
      getSnapshot: () => request("lens/questions/snapshot").then(() => structuredClone(current)),
      subscribe: (listener) => {
        if (typeof listener !== "function") throw new TypeError("A question listener is required.");
        if (listeners.size >= 64) throw new Error("Too many question subscribers.");
        listener(structuredClone(current));
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
      clearSelection: () =>
        request("lens/questions/clear-selection").then(() => structuredClone(current)),
    }),
    writable: false,
    configurable: false,
  });
  // The helper is injected before author scripts; subscription is synchronously available.
  request("lens/questions/snapshot").catch(() => {
    update({ ...current, state: "expired", message: "The App connection is unavailable." });
  });
})();
// Trusted click routing only. The native Host validates display ownership and opens HTTP(S).
(() => {
  let sequence = 0;
  document.addEventListener("pointerup", (event) => {
    if (!event.isTrusted) return;
    const selected = document.getSelection?.()?.toString().trim();
    const figure =
      event.target instanceof Element
        ? event.target.closest("figure, img, svg, [role='img']")
        : null;
    const selection = (
      selected ||
      figure?.getAttribute("aria-label") ||
      figure?.getAttribute("alt") ||
      figure?.getAttribute("title") ||
      figure?.textContent ||
      ""
    )
      .trim()
      .slice(0, 4000);
    if (!selection) return;
    parent.postMessage(
      {
        jsonrpc: "2.0",
        id: `lens-selection-${++sequence}`,
        method: "ui/update-model-context",
        params: { structuredContent: { lensSelection: selection } },
      },
      "*",
    );
  });
  document.addEventListener("click", (event) => {
    if (!event.isTrusted || event.defaultPrevented || event.button !== 0) return;
    const anchor = event.target instanceof Element ? event.target.closest("a[href]") : null;
    if (!anchor || anchor.hasAttribute("download")) return;
    let url;
    try {
      const href = anchor.getAttribute("href");
      if (href.trim().startsWith("#")) return;
      url = new URL(href, document.baseURI);
      const current = new URL(document.URL);
      if (
        url.origin === current.origin &&
        url.pathname === current.pathname &&
        url.search === current.search
      )
        return;
    } catch {
      return;
    }
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) return;
    event.preventDefault();
    parent.postMessage(
      {
        jsonrpc: "2.0",
        id: `lens-link-${++sequence}`,
        method: "ui/open-link",
        params: { url: url.href },
      },
      "*",
    );
  });
})();
