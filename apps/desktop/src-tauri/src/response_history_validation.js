(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const report = (payload) =>
    invoke("plugin:event|emit", {
      event: "lens-response-history-result",
      payload,
    });
  const root = () => document.querySelector("lens-overlay-view")?.shadowRoot;
  const notificationShown = () =>
    Boolean(root()?.querySelector(".lens-progress-snackbar .lens-response-update"));
  const pendingCount = () =>
    Number(root()?.querySelector("#lens-progress-notification")?.dataset.pendingCount ?? 0);
  const toggleCount = () =>
    root()?.querySelector(".overlay-status-toggle .overlay-new-response-count")?.textContent ?? "";
  const checks = {};
  const wait = async (predicate, stage = "DOM readiness") => {
    const deadline = performance.now() + 25000;
    while (!predicate()) {
      if (performance.now() > deadline) throw new Error(`Native ${stage} deadline exceeded`);
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  };
  try {
    await wait(() => root()?.querySelector("[data-response-id]"));
    const retained = root().querySelector("[data-response-id]");
    await wait(() => root().querySelector("iframe")?.srcdoc.includes("Retained visual 1"));
    const retainedFrame = root().querySelector("iframe");
    checks.initialSnapshotQuiet =
      !root().querySelector(".lens-response-update") && !toggleCount() && pendingCount() === 0;
    const output = root().querySelector(".lens-output");
    output.style.scrollSnapType = "none";
    const narrative = root().querySelector(".lens-output-narrative");
    output.scrollTop = narrative.offsetTop;
    await wait(() =>
      retained.querySelector("lens-markdown")?.textContent.includes("Native cumulative output"),
    );
    await new Promise((resolve) => setTimeout(resolve, 300));
    const markdown = retained.querySelector("lens-markdown");
    output.scrollTop =
      narrative.getBoundingClientRect().top -
      output.getBoundingClientRect().top +
      output.scrollTop +
      150;
    await new Promise((resolve) => setTimeout(resolve, 150));
    const readingPosition = output.scrollTop;
    const initialBottomGap = output.scrollHeight - output.clientHeight - readingPosition;
    await invoke("plugin:event|emit", { event: "lens-response-history-ready", payload: true });
    await wait(() => root()?.querySelectorAll("[data-response-id]").length === 3);
    await wait(
      () => pendingCount() === 2 && notificationShown(),
      "two appended response notification",
    );
    await new Promise((resolve) => setTimeout(resolve, 500));
    const positionRetained = Math.abs(output.scrollTop - readingPosition) <= 3;
    const frameRetained = root().querySelector("iframe") === retainedFrame;
    const markdownRetained = retained.querySelector("lens-markdown") === markdown;
    const block = await invoke("get_response_block", {
      operationId: "__OPERATION_ID__",
      representationId: "__REPRESENTATION_ID__",
      blockIndex: 1,
    });
    const html = await invoke("get_html_output", {
      operationId: "__OPERATION_ID__",
      representationId: "__REPRESENTATION_ID__",
      resourceId: "retained-validation-html",
    });
    let rejected = false;
    try {
      await invoke("get_response_block", {
        operationId: "00000000-0000-0000-0000-000000000099",
        representationId: "__REPRESENTATION_ID__",
        blockIndex: 1,
      });
    } catch {
      rejected = true;
    }
    Object.assign(checks, {
      threeResponses: root().querySelectorAll("[data-response-id]").length === 3,
      heroCount: Number(root().querySelector(".output-media-hero").dataset.count) >= 3,
      htmlFrameRetained: frameRetained,
      markdownRenderedAndRetained:
        markdownRetained && markdown.textContent.includes("Native cumulative output"),
      readingPositionRetained: positionRetained && initialBottomGap > 100,
      oldResponseDOM: root().querySelector("[data-response-id]") === retained,
      oldMarkdownIPC: block.type === "markdown" && block.text.includes("Response 1"),
      oldHTMLIPC: html.includes("Retained visual 1"),
      wrongOperationRejected: rejected,
      twoAppendsCounted: pendingCount() === 2,
      appendedNotificationShown: notificationShown(),
    });
    root().querySelector("#source-tab").click();
    await wait(() => root().querySelector("#source-tab")?.getAttribute("aria-selected") === "true");
    root().querySelector("#interpretation-tab").click();
    await wait(() =>
      retained.querySelector("lens-markdown")?.textContent.includes("Native cumulative output"),
    );
    await new Promise((resolve) => setTimeout(resolve, 500));
    checks.tabReadingPositionRetained =
      Math.abs(root().querySelector(".lens-output").scrollTop - readingPosition) <= 3;
    root().querySelector(".lens-progress-dismiss").click();
    await wait(
      () =>
        root().querySelector(".overlay-status-toggle")?.getAttribute("aria-expanded") === "false" &&
        !notificationShown(),
      "notification dismissal",
    );
    checks.dismissPreservesPendingCount = pendingCount() === 2 && /\b2\b/.test(toggleCount());
    checks.dismissPreservesReadingPosition = Math.abs(output.scrollTop - readingPosition) <= 3;
    root().querySelector(".overlay-status-toggle").click();
    await wait(
      () =>
        root().querySelector(".overlay-status-toggle")?.getAttribute("aria-expanded") === "true" &&
        notificationShown(),
      "notification toggle reopen",
    );
    checks.toggleReopensPendingNotification = pendingCount() === 2;
    const latest = root().querySelector('[data-response-sequence="3"]');
    root().querySelector(".lens-view-latest").click();
    await wait(
      () =>
        latest.querySelector("lens-markdown")?.textContent.includes("Native cumulative output") &&
        latest.querySelector("lens-markdown")?.textContent.includes("Response 3") &&
        root().activeElement === latest,
      "View latest response 3 navigation",
    );
    await wait(
      () =>
        pendingCount() === 0 && !toggleCount() && !root().querySelector(".lens-response-update"),
      "View latest acknowledgement",
    );
    const latestOffset = latest.getBoundingClientRect().top - output.getBoundingClientRect().top;
    checks.viewLatestTargetsResponseThree = latest.dataset.responseSequence === "3";
    checks.viewLatestRevealsNarrative = Math.abs(latestOffset) <= 3;
    checks.viewLatestFocusesTarget = root().activeElement === latest;
    checks.viewLatestAcknowledgesPending = pendingCount() === 0 && !toggleCount();
    await report({
      passed: Object.values(checks).every(Boolean),
      checks,
      readingPosition,
      initialBottomGap,
      actualPosition: output.scrollTop,
      clientHeight: output.clientHeight,
      scrollHeight: output.scrollHeight,
      latestOffset,
    });
  } catch (error) {
    await report({
      passed: false,
      error: String(error),
      checks,
      pendingCount: pendingCount(),
      toggleCount: toggleCount(),
      notificationShown: notificationShown(),
    });
  }
})();
