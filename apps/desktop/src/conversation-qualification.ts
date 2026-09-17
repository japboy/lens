// Development-only fixture, deliberately absent from PAGE_ENTRIES and production bundles.
import { LensSessionDocument } from "./components/lens-session-document";
import type { DocumentBlock } from "./application/session-document";
const view = new LensSessionDocument();
view.identity = "qualification-4096";
view.document = {
  entries: Array.from({ length: 4096 }, (_, index) => ({
    id: String(index),
    kind: "message",
    role: index % 2 ? "assistant" : "user",
    blocks: [
      {
        type: "deferred",
        entry_id: String(index),
        block_index: 0,
        revision: 1,
        content_type: index % 16 === 0 ? "html" : "markdown",
        byte_length: 4096,
      },
    ],
  })),
};
let loads = 0;
view.loadBlock = async (block) => {
  loads++;
  const index = Number(block.entry_id);
  return index % 16 === 0
    ? { type: "html", text: `<h1>Artifact ${index}</h1><p>Variable-height transcript fixture.</p>` }
    : {
        type: "markdown",
        text: `## Message ${index}\n\n${"A paragraph of **formatted** conversation content.\n\n".repeat((index % 12) + 1)}`,
      };
};
const mount = document.querySelector("#mount")!;
mount.append(view);
const virtualizer = () => view.shadowRoot?.querySelector("lit-virtualizer");
for (const [id, index] of [
  ["first", 0],
  ["middle", 4096],
  ["last", 8191],
] as const) {
  document
    .querySelector(`#${id}`)!
    .addEventListener("click", () =>
      virtualizer()?.element(index)?.scrollIntoView({ block: "start" }),
    );
}
document
  .querySelector("#tab")!
  .addEventListener("click", () => (view.isConnected ? view.remove() : mount.append(view)));
document.querySelector("#reset")!.addEventListener("click", () => {
  maxFrameGap = 0;
  frames = 0;
  last = performance.now();
});
document.querySelector("#huge")!.addEventListener("click", () => {
  maxFrameGap = 0;
  frames = 0;
  last = performance.now();
  view.identity = "qualification-huge";
  const block: DocumentBlock = {
    type: "markdown",
    text: "A large paragraph with **formatting**. ".repeat(28000),
  };
  view.document = {
    entries: [{ id: "huge", kind: "message", role: "assistant", blocks: [block] }],
  };
});
let frames = 0,
  last = performance.now(),
  maxFrameGap = 0;
function frame(now: number): void {
  frames++;
  maxFrameGap = Math.max(maxFrameGap, now - last);
  last = now;
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
setInterval(() => {
  const list = virtualizer();
  document.querySelector("#metrics")!.textContent = JSON.stringify(
    {
      entries: view.document?.entries.length,
      mountedRows: list?.children.length,
      bodyLoads: loads,
      scrollTop: list?.scrollTop,
      scrollHeight: list?.scrollHeight,
      frames,
      maxFrameGapMs: Math.round(maxFrameGap),
      connected: view.isConnected,
    },
    null,
    2,
  );
}, 250);
