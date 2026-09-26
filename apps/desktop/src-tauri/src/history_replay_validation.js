(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const root = () => document.querySelector("lens-overlay-view")?.shadowRoot;
  const wait = async (predicate) => {
    const deadline = performance.now() + 25000;
    while (!predicate()) {
      if (performance.now() > deadline)
        throw new Error("History replay DOM readiness deadline exceeded");
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  };
  try {
    const view = await invoke("get_session_view");
    await wait(() => root()?.querySelectorAll("[data-response-id]").length === 3);
    const initialHistoryQuiet =
      !root().querySelector(".lens-response-update") &&
      !root().querySelector(".overlay-new-response-count");
    await wait(() => root()?.querySelector("iframe")?.srcdoc.includes("Replay visual 1"));
    const firstFrame = root().querySelector("iframe");
    root().querySelector('[aria-label="Next media"]').click();
    await wait(() =>
      root()
        ?.querySelector('.output-media-slide[aria-hidden="false"] iframe')
        ?.srcdoc.includes("Replay visual 2"),
    );
    const secondFrame = root().querySelector('.output-media-slide[aria-hidden="false"] iframe');
    const output = root().querySelector(".lens-output");
    for (const section of root().querySelectorAll("[data-response-id]")) {
      output.scrollTop += section.getBoundingClientRect().top - output.getBoundingClientRect().top;
      await wait(() =>
        section
          .querySelector("lens-markdown")
          ?.textContent.includes(`Replay body #${section.dataset.responseSequence}`),
      );
    }
    const checks = {
      initialHistoryHasNoNewResponseNotification: initialHistoryQuiet,
      threeHistoricalResponses: root().querySelectorAll("[data-response-id]").length === 3,
      bothHTMLArtifacts: Number(root().querySelector(".output-media-hero").dataset.count) === 2,
      secondHTMLAvailable:
        firstFrame !== secondFrame && secondFrame.srcdoc.includes("Replay visual 2"),
      textOnlyLaterResponse:
        root()
          .querySelector('[data-response-sequence="3"] lens-markdown')
          ?.textContent.includes("Replay body #3") === true,
      lightweightSnapshot:
        !JSON.stringify(view).includes("Replay body #") &&
        !JSON.stringify(view).includes("<h1>Replay visual"),
      noFailedMedia: !root()
        .querySelector(".output-media-hero")
        .textContent.includes("Failed content"),
    };
    root().querySelector("#conversation-tab").click();
    await wait(() => root().querySelector("lens-session-document"));
    checks.conversationAvailable = true;
    root().querySelector("#interpretation-tab").click();
    await wait(() => root()?.querySelectorAll("[data-response-id]").length === 3);
    checks.interpretationRestored = true;
    checks.historyTabRestoreHasNoNewResponseNotification =
      !root().querySelector(".lens-response-update") &&
      !root().querySelector(".overlay-new-response-count");
    await invoke("plugin:event|emit", {
      event: "lens-history-replay-result",
      payload: { passed: Object.values(checks).every(Boolean), checks },
    });
  } catch (error) {
    await invoke("plugin:event|emit", {
      event: "lens-history-replay-result",
      payload: { passed: false, error: String(error) },
    });
  }
})();
