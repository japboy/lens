import fontAwesomeStyles from "@fortawesome/fontawesome-free/css/fontawesome.css?inline";
import fontAwesomeSolidStyles from "@fortawesome/fontawesome-free/css/solid.css?inline";
import { css, unsafeCSS } from "lit";
import applicationStyles from "../styles.css?inline";

export const viewHostStyles = css`
  :host {
    display: block;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }

  lens-agent-settings,
  lens-target-card,
  lens-agent-output,
  lens-media-gallery,
  lens-extraction-diagnostics {
    display: contents;
  }

  lens-prompt-settings {
    display: block;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
`;

export const sharedApplicationStyles = unsafeCSS(applicationStyles);
export const sharedIconStyles = [unsafeCSS(fontAwesomeStyles), unsafeCSS(fontAwesomeSolidStyles)];
