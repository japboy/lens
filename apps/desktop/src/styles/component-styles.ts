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

  input[type="text"],
  input:not([type]) {
    appearance: auto;
    color: FieldText;
    background: Field;
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
