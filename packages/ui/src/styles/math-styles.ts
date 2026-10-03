import katexStyles from "katex/dist/katex.css?inline";
import { css, unsafeCSS } from "lit";

// Only the pinned, bundled stylesheet enters unsafeCSS. User TeX never does.
export const mathStyles = [
  unsafeCSS(katexStyles),
  css`
    .katex-display {
      max-width: 100%;
      overflow-x: auto;
      overflow-y: hidden;
      padding-block: 0.25em;
    }

    .katex-display > .katex {
      text-align: start;
    }
  `,
];
