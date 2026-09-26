import { css } from "lit";

export const viewHostStyles = css`
  [data-region-error]:empty {
    display: none;
  }

  :host {
    display: block;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
`;

const textEntrySelector = css`[data-lens-control="text-entry"]:is(
  input:not([type]),
  input:is([type="text"], [type="search"], [type="url"], [type="tel"], [type="email"], [type="password"], [type="number"]),
  textarea
)`;

const actionButtonSelector = css`[data-lens-button-role]:is(button):where(
  [data-lens-button-role="normal"],
  [data-lens-button-role="primary"],
  [data-lens-button-role="cancel"],
  [data-lens-button-role="destructive"]
)`;

export const controlStyles = css`
  * {
    box-sizing: border-box;
  }

  button,
  input,
  select,
  textarea {
    font: inherit;
  }

  button {
    appearance: auto;
    cursor: default;
    min-height: 24px;
    max-width: 100%;
    white-space: normal;
  }

  ${actionButtonSelector} {
    font-weight: 400;
    appearance: var(--button-appearance, auto);
    background: var(--button-fill, revert);
    color: var(--button-foreground, revert);
    border: var(--button-border, revert);
    border-radius: var(--button-radius, revert);
  }

  ${actionButtonSelector}[data-lens-button-style="borderless"] {
    background: var(--button-borderless-fill, revert);
    border-color: var(--button-borderless-border, revert);
  }

  ${actionButtonSelector}:hover:not(:disabled) {
    background: var(--button-hover-fill, revert);
    border-color: var(--button-hover-border, revert);
  }

  ${actionButtonSelector}:active:not(:disabled) {
    background: var(--button-pressed-fill, revert);
    border-color: var(--button-pressed-border, revert);
    box-shadow: var(--button-pressed-shadow, revert);
  }

  ${actionButtonSelector}[data-lens-button-role="primary"] {
    background: var(--button-primary-fill, revert);
    color: var(--button-primary-foreground, revert);
  }

  ${actionButtonSelector}[data-lens-button-role="primary"]:hover:not(:disabled) {
    background: var(--button-primary-hover-fill, revert);
    border-color: var(--button-pressed-border, revert);
  }

  ${actionButtonSelector}[data-lens-button-role="primary"]:active:not(:disabled) {
    background: var(--button-primary-pressed-fill, revert);
  }

  ${actionButtonSelector}[data-lens-button-role="destructive"] {
    color: var(--button-destructive-foreground, revert);
  }

  ${actionButtonSelector}:disabled {
    background: var(--button-disabled-fill, revert);
    color: var(--button-disabled-foreground, revert);
  }

  ${actionButtonSelector}:focus-visible {
    outline: var(--button-focus-outline, revert);
    outline-offset: 1px;
  }

  ${textEntrySelector} {
    appearance: auto;
    color: var(--control-foreground, FieldText);
    background: var(--control-background, Field);
  }

  :host([data-platform="macos"]) ${textEntrySelector} {
    appearance: none;
    border: 1px solid var(--control-border, ButtonBorder);
    border-radius: 5px;
  }

  :host([data-platform="macos"]) ${textEntrySelector}:disabled {
    border-color: color-mix(in srgb, var(--control-border, ButtonBorder) 65%, transparent);
    color: GrayText;
    cursor: default;
  }

  :host([data-platform="macos"]) ${textEntrySelector}:focus-visible {
    border-color: var(--control-focus-ring, Highlight);
    outline: 3px solid var(--control-focus-ring, Highlight);
    outline-offset: 1px;
  }

  :host([data-platform="macos"][data-increase-contrast="true"]) ${textEntrySelector} {
    border-width: 2px;
    border-color: CanvasText;
  }

  :host([data-platform="macos"][data-increase-contrast="true"]) ${textEntrySelector}:focus-visible {
    border-color: AccentColor;
    outline-width: 4px;
  }

  @media (prefers-contrast: more) {
    :host([data-platform="macos"]) ${textEntrySelector} {
      border-width: 2px;
      border-color: CanvasText;
    }

    :host([data-platform="macos"]) ${textEntrySelector}:focus-visible {
      border-color: AccentColor;
      outline-width: 4px;
    }
  }

  h2 {
    margin: 0 0 10px;
    font-size: 13px;
    font-weight: 650;
  }

  p {
    line-height: 1.45;
  }
`;

export const feedbackStyles = css`
  .help {
    margin: 7px 0 0;
    color: GrayText;
    font-size: 12px;
  }

  .error {
    color: MarkText;
    background: Mark;
  }
`;

export const accessibilityStyles = css`
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
`;

export const reducedMotionStyles = css`
  @media (prefers-reduced-motion: reduce) {
    *,
    *::before,
    *::after {
      animation-duration: 0.001ms !important;
      animation-iteration-count: 1 !important;
    }
  }
`;
