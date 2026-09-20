import { writeFileSync } from "node:fs";
import { render } from "@lit-labs/ssr";
import { html } from "lit";
import "../../src/components/lens-about-view";
import "../../src/components/lens-settings-view";
import "../../src/components/lens-overlay-view";
import "../../src/components/lens-target-selection-view";

import "../../src/components/lens-settings-recovery-view";

const templates = {
  "settings-recovery": html`<lens-settings-recovery-view
    defer-hydration
  ></lens-settings-recovery-view>`,
  about: html`<lens-about-view defer-hydration></lens-about-view>`,
  settings: html`<lens-settings-view defer-hydration></lens-settings-view>`,
  overlay: html`<lens-overlay-view defer-hydration></lens-overlay-view>`,
  "target-selection": html`<lens-target-selection-view
    defer-hydration
  ></lens-target-selection-view>`,
};
const result = Object.fromEntries(
  Object.entries(templates).map(([view, template]) => {
    const first = Array.from(render(template)).join("");
    const second = Array.from(render(template)).join("");
    if (first !== second) throw new Error(`Non-deterministic initial view: ${view}`);
    if (!first.includes('shadowrootmode="open"')) throw new Error(`Missing DSD: ${view}`);
    return [view, first];
  }),
);
const output = process.argv[2];
if (!output) throw new Error("Missing prerender output path");
writeFileSync(output, JSON.stringify(result));
