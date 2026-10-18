// Opt-in debug acceptance probe for the provider session opened by the normal catalog path.
(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const root = () => document.querySelector("lens-overlay-view")?.shadowRoot;
  const wait = async (label, predicate) => {
    const deadline = performance.now() + 30000;
    while (!predicate()) {
      if (performance.now() > deadline)
        throw new Error(`${label}: DOM readiness deadline exceeded`);
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  };
  const visible = (element) => {
    if (!element) return false;
    const rect = element.getBoundingClientRect();
    return (
      rect.width > 0 &&
      rect.height > 0 &&
      rect.bottom > 0 &&
      rect.right > 0 &&
      rect.top < innerHeight &&
      rect.left < innerWidth &&
      getComputedStyle(element).visibility !== "hidden"
    );
  };
  const deepText = (element) => {
    if (!element) return "";
    let text = element.textContent ?? "";
    for (const child of element.querySelectorAll("*")) {
      if (child.shadowRoot) text += deepText(child.shadowRoot);
    }
    return text;
  };
  const checks = {};
  const counts = {};
  try {
    const view = await invoke("get_session_view");
    if (view.phase !== "ready") throw new Error("Provider history is not ready");
    const responses = view.interpretation?.responses ?? [];
    const textResponses = responses.filter((response) =>
      response.blocks.some((block) => block.type === "markdown" && block.byte_length > 0),
    );
    const html = responses
      .flatMap((response) => response.blocks)
      .filter((block) => block.type === "html");
    counts.responses = responses.length;
    counts.textResponses = textResponses.length;
    counts.htmlArtifacts = html.length;
    counts.conversationEntries = view.conversation?.entries.length ?? 0;
    checks.providerContentPresent = responses.length > 0 && textResponses.length > 0;
    checks.validatedHtmlPresent = html.length > 0;
    await wait(
      "Interpretation responses",
      () => root()?.querySelectorAll("[data-response-id]").length === responses.length,
    );
    checks.interpretationVisible = visible(root().querySelector(".lens-output"));
    await wait(
      "HTML Hero",
      () => Number(root()?.querySelector(".output-media-hero")?.dataset.count) >= html.length,
    );
    await wait("HTML frame", () =>
      [...root().querySelectorAll("iframe")].some(
        (frame) => visible(frame) && (frame.srcdoc.length > 100 || frame.src.startsWith("http")),
      ),
    );
    checks.htmlFrameVisible = true;
    await wait("HTML App handshake", () =>
      [...root().querySelectorAll("lens-mcp-app")].some((app) => app.stage === "ready"),
    );
    checks.htmlAppHandshakeReady = true;
    const output = root().querySelector(".lens-output");
    let lastAnswer;
    let lastAnswerSource;
    for (const response of textResponses) {
      const textBlocks = response.blocks.filter(
        (item) => item.type === "markdown" && item.byte_length > 0,
      );
      for (const block of textBlocks) {
        const body = await invoke("get_session_block", {
          request: { generation: view.generation, ...block.source },
        });
        if (body.block.type !== "markdown" || !body.block.text.trim())
          throw new Error("Historical answer block is empty");
        const section = [...root().querySelectorAll("[data-response-id]")].find(
          (item) => item.dataset.responseId === response.response_id,
        );
        if (!section) throw new Error("Historical response section is absent");
        output.scrollTop +=
          section.getBoundingClientRect().top - output.getBoundingClientRect().top;
        await wait("Lazy historical answer", () =>
          [...section.querySelectorAll("lens-markdown")].some(
            (markdown) =>
              markdown.state?.markdown === body.block.text &&
              visible(markdown) &&
              Boolean(markdown.textContent.trim()),
          ),
        );
        lastAnswer = body.block.text;
        lastAnswerSource = block.source;
        counts.materializedTextBlocks = (counts.materializedTextBlocks ?? 0) + 1;
      }
    }
    checks.allHistoricalAnswersMaterialized = textResponses.length > 0;
    root().querySelector("#conversation-tab").click();
    await wait("Conversation", () =>
      root()?.querySelector("lens-session-document")?.shadowRoot?.querySelector("lit-virtualizer"),
    );
    const conversation = root().querySelector("lens-session-document");
    const virtualizer = conversation.shadowRoot.querySelector("lit-virtualizer");
    await virtualizer.updateComplete;
    await virtualizer.layoutComplete;
    counts.conversationRows = virtualizer.items.length;
    checks.conversationVisible = visible(virtualizer);
    checks.conversationStartsBeforeFinalAnswer = !deepText(conversation.shadowRoot).includes(
      lastAnswer.slice(0, 80),
    );
    const rowId = `${lastAnswerSource.entry_id}:${lastAnswerSource.block_index}`;
    const rowIndex = virtualizer.items.findIndex((row) => row.id === rowId);
    if (rowIndex < 0) throw new Error("Final answer Conversation row is absent");
    const target = virtualizer.element(rowIndex);
    if (!target) throw new Error("Final answer Conversation scroll target is absent");
    target.scrollIntoView({ block: "end" });
    await virtualizer.layoutComplete;
    // Large preceding rows are measured asynchronously. Keep the exact row pinned
    // during the bounded wait instead of trusting its first estimated position.
    await wait("Final Conversation answer viewport", () => {
      virtualizer.element(rowIndex)?.scrollIntoView({ block: "end" });
      const row = [...virtualizer.children].find((element) => element.dataset.rowId === rowId);
      const text = row?.shadowRoot?.querySelector("lens-conversation-text");
      if (text?.text !== lastAnswer) return false;
      const tail = text.shadowRoot?.querySelector(".chunk:last-child");
      if (!visible(tail)) return false;
      const content = tail.getBoundingClientRect();
      const viewport = virtualizer.getBoundingClientRect();
      return (
        Boolean(tail.textContent.trim()) &&
        content.bottom > viewport.top &&
        content.top < viewport.bottom
      );
    });
    checks.finalConversationAnswerVisible = true;
    root().querySelector("#interpretation-tab").click();
    await wait(
      "Interpretation restored",
      () => root()?.querySelectorAll("[data-response-id]").length === responses.length,
    );
    checks.interpretationRestored = true;
    root().querySelector(".lens-output").scrollTop = 0;
    // Initial Conversation position is diagnostic, not an acceptance requirement.
    const required = Object.entries(checks).filter(
      ([name]) => name !== "conversationStartsBeforeFinalAnswer",
    );
    await invoke("plugin:event|emit", {
      event: "lens-provider-history-result",
      payload: { passed: required.every(([, value]) => value), checks, counts },
    });
  } catch (error) {
    await invoke("plugin:event|emit", {
      event: "lens-provider-history-result",
      payload: { passed: false, error: String(error), checks, counts },
    });
  }
})();
