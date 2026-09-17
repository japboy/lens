// @vitest-environment jsdom
import { expect, it } from "vitest";
import type { LensOverlayView } from "./components/lens-overlay-view";
import type { LensAgentOutput } from "./components/lens-agent-output";

it("registers and mounts real overlays and media through the qualification entrypoint", async () => {
  window.matchMedia ??= () =>
    ({
      matches: false,
      addEventListener() {},
      removeEventListener() {},
    }) as unknown as MediaQueryList;
  document.body.innerHTML =
    '<select id="case"><option>single</option><option>mixed</option></select><lens-overlay-view id="normal"></lens-overlay-view><lens-overlay-view id="history"></lens-overlay-view>';
  await import("./media-qualification");
  for (const id of ["normal", "history"]) {
    const overlay = document.querySelector<LensOverlayView>(`#${id}`)!;
    await overlay.updateComplete;
    expect(overlay.shadowRoot?.querySelector(".overlay-shell")).not.toBeNull();
    const output = overlay.shadowRoot!.querySelector<LensAgentOutput>("lens-agent-output")!;
    expect(output).not.toBeNull();
    await output.updateComplete;
    expect(output.querySelector("lens-output-media")).not.toBeNull();
  }
  for (const name of [
    "lens-overlay-view",
    "lens-agent-output",
    "lens-output-media",
    "lens-markdown",
    "lens-session-document",
  ])
    expect(customElements.get(name)).toBeDefined();
  document.body.replaceChildren();
});
