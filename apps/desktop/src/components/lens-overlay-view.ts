import { initialOverlayState } from "../rendering/initial-state";
import { renderSnapshotFailure } from "../rendering/snapshot-status";
import { LitElement, css, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import appIconUrl from "../../src-tauri/icons/icon-macos.svg?url";
import type { OverlayViewModel } from "../application/view-models";
import type { LensRepresentation, LensState } from "../types";
import { composeOutputMedia } from "../output-media";
import type { HtmlOutputContent } from "../application/html-output-controller";
import {
  accessibilityStyles,
  controlStyles,
  feedbackStyles,
  reducedMotionStyles,
  viewHostStyles,
} from "../styles/component-styles";
import { sharedIconStyles } from "../styles/icon-styles";
import {
  lensLiveStatus,
  lensOutputPresentation,
  lensProgressSnackbar,
  lensSourceJson,
  STAGE_LABEL,
  supportedAuthMethods,
} from "../view-model";
import {
  dispatchComponentEvent,
  OVERLAY_INTENT_EVENT,
  type AgentOutputIntent,
  type OverlayIntent,
} from "./events";

const LENS_TABS = [
  { id: "interpretation", label: "Interpretation" },
  { id: "source", label: "Source" },
  { id: "diagnostics", label: "Diagnostics" },
] as const;

type LensTab = (typeof LENS_TABS)[number]["id"];

interface InterpretationScrollPosition {
  readonly top: number;
  readonly wasAtBottom: boolean;
  readonly hadMedia: boolean;
}

interface OverlayNotification {
  readonly title: string;
  readonly detail: string;
  readonly busy: boolean;
  readonly prominent: boolean;
}

function overlayNotification(lens: LensState): OverlayNotification | undefined {
  if (
    lens.session_controls?.active &&
    lens.session_controls.interactions.some((i) => i.status === "pending")
  ) {
    return {
      title: "Agent response required",
      detail: "Choose a response below.",
      busy: false,
      prominent: true,
    };
  }
  const liveStatus = lensLiveStatus(lens.live);
  if (lens.representation && lens.stage !== "transforming") return liveStatus;
  const progress = lensProgressSnackbar(lens.stage);
  return progress
    ? {
        ...progress,
        detail:
          lens.stage === "transforming"
            ? lens.agent?.progress_text || progress.detail
            : progress.detail,
        busy: true,
        prominent: true,
      }
    : liveStatus;
}

@customElement("lens-overlay-view")
export class LensOverlayView extends LitElement {
  static styles = [
    viewHostStyles,
    controlStyles,
    css`
      lens-agent-output,
      lens-media-gallery,
      lens-extraction-diagnostics {
        display: contents;
      }

      .overlay-shell {
        --output-media-cue-size: 32px;
        --progress-bottom-clearance: 0px;
        isolation: isolate;
        height: 100dvh;
        display: grid;
        grid-template-areas: "header" "source" "tabs" "content" "footer";
        grid-template-columns: minmax(0, 1fr);
        grid-template-rows: auto auto auto minmax(0, 1fr) auto;
        overflow: hidden;
        border: 1px solid Separator;
        border-radius: 12px;
        background: transparent;
        color: CanvasText;
        box-shadow: 0 12px 36px color-mix(in srgb, CanvasText 20%, transparent);
      }

      .overlay-shell[data-media-cue="true"] {
        --progress-bottom-clearance: var(--output-media-cue-size);
      }

      .overlay-header {
        grid-area: header;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        min-height: 44px;
        padding: 7px 14px;
      }

      .overlay-brand,
      .overlay-header-actions {
        display: flex;
        align-items: center;
        min-width: 0;
      }

      .overlay-brand {
        flex: 1 1 auto;
      }

      .overlay-app-icon {
        flex: 0 0 auto;
        display: block;
        width: 28px;
        height: 28px;
        object-fit: contain;
      }

      .overlay-header-actions {
        flex: 0 0 auto;
        gap: 6px;
      }

      .overlay-header-action {
        appearance: auto;
        min-height: 26px;
        padding-inline: 9px;
        font: inherit;
        font-size: 12px;
        white-space: nowrap;
      }

      .overlay-header-action:disabled {
        cursor: default;
      }

      .close-button:focus-visible {
        outline: 2px solid AccentColor;
        outline-offset: 1px;
      }

      .overlay-source-summary {
        position: relative;
        grid-area: source;
        display: flex;
        align-items: center;
        gap: 10px;
        min-width: 0;
        padding: 9px 14px;
        border-block: 1px solid color-mix(in srgb, Separator 82%, transparent);
        background: color-mix(in srgb, AccentColor 8%, transparent);
      }

      .overlay-source-summary::before {
        position: absolute;
        inset-block: 7px;
        inset-inline-start: 0;
        width: 2px;
        border-radius: 0 999px 999px 0;
        background: AccentColor;
        content: "";
      }

      .overlay-source-icon {
        flex: 0 0 auto;
        display: grid;
        width: 26px;
        height: 26px;
        place-items: center;
        border-radius: 7px;
        color: AccentColor;
        background: color-mix(in srgb, AccentColor 12%, transparent);
        font-size: 12px;
      }

      .overlay-source-copy {
        display: grid;
        min-width: 0;
        gap: 1px;
      }

      .overlay-source-count,
      .overlay-source-targets {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .overlay-source-count {
        color: CanvasText;
        font-size: 12px;
        font-weight: 650;
      }

      .overlay-source-targets {
        color: GrayText;
        font-size: 11px;
      }

      .overlay-main {
        grid-area: content;
        min-height: 0;
        display: flex;
        flex-direction: column;
        overflow: hidden;
      }

      .lens-progress-region {
        grid-area: content;
        align-self: end;
        min-width: 0;
        position: relative;
        z-index: 1;
        overflow: visible;
        display: flex;
        justify-content: center;
        pointer-events: none;
      }

      .overlay-shell[data-progress="true"] > .lens-progress-region {
        padding: 14px;
        margin-block-end: var(--progress-bottom-clearance);
      }

      .lens-progress-snackbar {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        align-items: center;
        gap: 10px;
        width: min(100%, 420px);
        min-height: 48px;
        padding: 9px 12px;
        overflow: hidden;
        border: 1px solid color-mix(in srgb, Separator 88%, transparent);
        border-radius: 12px;
        background: color-mix(in srgb, Canvas 82%, transparent);
        -webkit-backdrop-filter: blur(18px) saturate(150%);
        backdrop-filter: blur(18px) saturate(150%);
        box-shadow:
          0 10px 30px color-mix(in srgb, CanvasText 18%, transparent),
          0 1px 3px color-mix(in srgb, CanvasText 10%, transparent);
        animation: lens-progress-snackbar-enter 160ms ease-out both;
      }

      .lens-progress-snackbar[data-interactive="true"] {
        display: block;
        padding: 0;
        max-height: 100%;
        overflow-y: auto;
        pointer-events: auto;
      }

      .lens-progress-snackbar[data-interactive="true"] > .lens-status-announcement {
        padding: 9px 12px;
      }

      .agent-interaction fieldset {
        display: block;
        min-width: 0;
        margin: 0;
        padding: 0;
        border: 0;
      }

      .interaction-body {
        padding: 0 12px 12px 42px;
        max-height: min(40vh, 320px);
        overflow: auto;
        overflow-wrap: anywhere;
        font-size: 12px;
      }

      .interaction-body h3 {
        margin: 0 0 8px;
        font-size: 12px;
      }

      .interaction-body p {
        margin: 0 0 8px;
      }

      .interaction-body pre {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        font-size: 11px;
      }

      .interaction-body input,
      .interaction-body select {
        max-width: 100%;
      }

      .interaction-actions {
        display: flex;
        flex-wrap: wrap;
        justify-content: flex-end;
        gap: 8px;
        padding: 10px 12px;
        border-top: 1px solid color-mix(in srgb, Separator 88%, transparent);
      }

      .interaction-actions button {
        margin: 0;
        font-size: 12px;
      }

      .interaction-feedback {
        margin: 0;
        padding: 8px 12px;
        font-size: 11px;
      }

      @media (max-width: 380px) {
        .interaction-body {
          padding-left: 12px;
        }
        .interaction-actions {
          flex-direction: column-reverse;
        }
      }

      .lens-status-announcement {
        display: grid;
        grid-template-columns: auto minmax(0, 1fr);
        align-items: center;
        gap: 10px;
        min-width: 0;
      }

      .lens-progress-snackbar .fa-spinner,
      .lens-status-icon {
        --fa-animation-duration: 1.2s;
        width: 20px;
        color: AccentColor;
        font-size: 18px;
        text-align: center;
      }

      .lens-progress-copy {
        min-width: 0;
        display: grid;
        gap: 1px;
      }

      .lens-progress-copy strong,
      .lens-progress-copy span {
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .lens-progress-copy strong {
        color: CanvasText;
        font-size: 12px;
        font-weight: 650;
      }

      .lens-progress-copy span {
        color: GrayText;
        font-size: 11px;
      }

      @keyframes lens-progress-snackbar-enter {
        from {
          opacity: 0;
          transform: translateY(6px) scale(0.98);
        }
        to {
          opacity: 1;
          transform: translateY(0) scale(1);
        }
      }

      .overlay-footer {
        position: relative;
        grid-area: footer;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        min-height: 34px;
        padding: 5px 14px;
        border-top: 1px solid Separator;
      }

      .overlay-footer-status {
        flex: 1 1 auto;
        display: flex;
        align-items: center;
        min-width: 0;
        gap: 7px;
        color: GrayText;
        font-size: 12px;
      }

      .overlay-status-toggle {
        appearance: none;
        min-height: 24px;
        padding: 0;
        border: 0;
        border-radius: 4px;
        background: transparent;
        font: inherit;
        font-size: 12px;
        text-align: start;
      }

      .overlay-status-toggle:hover,
      .overlay-status-toggle:focus-visible,
      .overlay-status-toggle[aria-expanded="true"] {
        color: CanvasText;
      }

      .overlay-status-toggle:is(:hover, :focus-visible) .overlay-stage {
        text-decoration: underline;
        text-underline-offset: 2px;
      }

      .overlay-status-toggle:focus-visible,
      .lens-progress-dismiss:focus-visible {
        outline: 2px solid AccentColor;
        outline-offset: 2px;
      }

      .lens-progress-dismiss {
        pointer-events: auto;
      }

      .overlay-stage-indicator {
        flex: 0 0 auto;
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: AccentColor;
        box-shadow: 0 0 0 2px color-mix(in srgb, AccentColor 14%, transparent);
      }

      .overlay-stage {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }

      .close-button {
        appearance: none;
        display: grid;
        place-items: center;
        border: 0;
        border-radius: 50%;
        min-width: 24px;
        width: 24px;
        height: 24px;
        padding: 0;
        background: transparent;
        color: GrayText;
        line-height: 1;
      }

      .close-button .fa-xmark {
        font-size: 14px;
      }

      .close-button:hover {
        background: SelectedItem;
        color: SelectedItemText;
      }

      .quality {
        flex: 0 0 auto;
        border: 1px solid Separator;
        border-radius: 999px;
        padding: 1px 7px;
        font-variant-caps: all-small-caps;
      }

      .quality-full {
        color: LinkText;
      }

      .quality-partial {
        color: MarkText;
        background: Mark;
      }

      .quality-unavailable {
        color: GrayText;
      }

      .lens-tabs {
        grid-area: tabs;
        flex: 0 0 auto;
        min-width: 0;
        overflow-x: auto;
        margin: 0;
        padding: 0 14px;
        border-bottom: 1px solid Separator;
        scrollbar-width: none;
      }

      .lens-tabs::-webkit-scrollbar {
        display: none;
      }

      .lens-tabs > [role="tablist"] {
        display: flex;
        align-items: center;
        gap: 20px;
        min-width: max-content;
      }

      .lens-tab {
        appearance: none;
        flex: 0 0 auto;
        min-height: 36px;
        margin-bottom: -1px;
        padding: 1px 0 0;
        border: 0;
        border-bottom: 2px solid transparent;
        border-radius: 0;
        background: transparent;
        color: GrayText;
        font: inherit;
        font-size: 12px;
      }

      .lens-tab[aria-selected="true"] {
        border-bottom-color: AccentColor;
        color: CanvasText;
        font-weight: 600;
      }

      .lens-tab:hover {
        color: CanvasText;
      }

      .lens-tab:focus-visible {
        outline: 2px solid AccentColor;
        outline-offset: -3px;
      }

      .lens-panel {
        flex: 1 1 auto;
        min-height: 0;
        display: flex;
        flex-direction: column;
        overflow: hidden;
      }

      .lens-content {
        flex: 1 1 auto;
        min-height: 0;
        overflow: auto;
        padding: 10px 18px 24px;
        line-height: 1.55;
        user-select: text;
      }

      .source-view {
        display: flex;
        flex-direction: column;
        gap: 16px;
      }

      .source-json h2,
      .input-media-preview h2 {
        margin: 0;
        color: CanvasText;
        font-size: 13px;
      }

      .source-content {
        margin: 0;
        border: 1px solid Separator;
        border-radius: 8px;
        padding: 12px;
        background: color-mix(in srgb, CanvasText 3%, transparent);
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        tab-size: 2;
        font-family: ui-monospace, "SFMono-Regular", Menlo, Monaco, monospace;
        font-size: 12px;
      }

      .source-content code {
        font: inherit;
      }

      .source-json {
        display: flex;
        flex-direction: column;
        gap: 8px;
      }

      .input-media-preview {
        display: flex;
        flex-direction: column;
        gap: 8px;
      }

      .input-media-preview > header {
        display: flex;
        align-items: baseline;
        justify-content: space-between;
        gap: 12px;
        color: GrayText;
        font-size: 11px;
        font-variant-numeric: tabular-nums;
      }

      .input-media-carousel {
        display: grid;
        grid-template-columns: 28px minmax(0, 1fr) 28px;
        align-items: center;
        gap: 8px;
      }

      .input-media-carousel > button {
        display: grid;
        align-self: stretch;
        min-height: 58px;
        padding: 0;
        place-items: center;
        font-size: 22px;
        line-height: 1;
      }

      .input-media-thumbnails {
        display: flex;
        min-width: 0;
        margin: 0;
        padding: 2px 2px 8px;
        gap: 8px;
        overflow-x: auto;
        overscroll-behavior-inline: contain;
        scroll-snap-type: inline mandatory;
        scrollbar-width: thin;
        list-style: none;
      }

      .input-media-thumbnails li {
        flex: 0 0 auto;
        scroll-snap-align: start;
      }

      .input-media-thumbnail {
        position: relative;
        display: block;
        width: 76px;
        height: 56px;
        overflow: hidden;
        border: 2px solid transparent;
        border-radius: 7px;
        padding: 2px;
        background: color-mix(in srgb, CanvasText 6%, transparent);
      }

      .input-media-thumbnail.is-selected {
        border-color: AccentColor;
        box-shadow: 0 0 0 1px color-mix(in srgb, AccentColor 45%, transparent);
      }

      .input-media-thumbnail img {
        display: block;
        width: 100%;
        height: 100%;
        border-radius: 3px;
        object-fit: contain;
      }

      .input-media-thumbnail span {
        position: absolute;
        right: 3px;
        bottom: 3px;
        min-width: 16px;
        border-radius: 999px;
        padding: 1px 4px;
        color: Canvas;
        background: color-mix(in srgb, CanvasText 82%, transparent);
        font-size: 10px;
        font-variant-numeric: tabular-nums;
        line-height: 14px;
        text-align: center;
      }

      .input-media-carousel,
      .input-media-thumbnails,
      .input-media-thumbnail {
        min-width: 0;
      }

      .input-media-preview figure {
        margin: 0;
        overflow: hidden;
        border: 1px solid Separator;
        border-radius: 8px;
        background: color-mix(in srgb, CanvasText 4%, transparent);
      }

      .input-media-preview img {
        display: block;
        width: 100%;
        max-height: min(48vh, 520px);
        object-fit: contain;
        background: color-mix(in srgb, CanvasText 7%, transparent);
      }

      .input-media-preview figcaption {
        border-top: 1px solid Separator;
        padding: 8px 10px;
        color: GrayText;
        font-size: 11px;
      }

      .input-media-metadata {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
        margin: 0;
        gap: 8px 16px;
      }

      .input-media-metadata div {
        min-width: 0;
      }

      .input-media-metadata dt {
        margin-bottom: 2px;
        color: GrayText;
      }

      .input-media-metadata dd {
        margin: 0;
        overflow-wrap: anywhere;
        color: CanvasText;
        font-variant-numeric: tabular-nums;
      }

      .input-media-error {
        margin: 0;
        padding: 24px;
        color: CanvasText;
        text-align: center;
      }

      .lens-output,
      .lens-output-narrative {
        display: flex;
        flex-direction: column;
        gap: 16px;
      }

      .lens-output.has-media {
        --output-media-cue-height: 0px;
        position: relative;
        display: block;
        container-type: size;
        padding: 0;
        scrollbar-gutter: stable;
        overscroll-behavior-y: contain;
      }

      .lens-output.has-media.has-narrative {
        --output-media-cue-height: var(--output-media-cue-size);
      }

      .lens-output.has-media .lens-output-narrative {
        padding: 22px 18px 28px;
      }

      lens-output-media {
        display: block;
        min-width: 0;
      }

      .output-media-hero {
        display: grid;
        grid-template-rows: auto minmax(144px, 1fr);
        height: max(144px, calc(100cqh - var(--output-media-cue-height)));
        min-height: min-content;
        overflow: hidden;
        background: color-mix(in srgb, Canvas 94%, CanvasText);
      }

      .output-media-stage {
        grid-row: 2;
        position: relative;
        isolation: isolate;
        min-width: 0;
        min-height: 0;
        overflow: hidden;
      }

      .output-media-error {
        flex: 0 0 auto;
        margin: 0;
        padding: 8px 12px;
        background: Canvas;
        color: CanvasText;
        font-size: 12px;
        line-height: 1.4;
        overflow-wrap: anywhere;
      }

      .output-media-ambient {
        position: absolute;
        z-index: -1;
        inset: -48px;
        pointer-events: none;
        opacity: 0.55;
      }

      .output-media-ambient img {
        display: block;
        width: 100%;
        height: 100%;
        object-fit: cover;
        filter: blur(38px) saturate(0.85);
        transform: scale(1.15);
      }

      .output-media-rail {
        display: flex;
        height: 100%;
        overflow: auto hidden;
        scroll-snap-type: x mandatory;
        overscroll-behavior-x: contain;
        scrollbar-width: none;
      }

      .output-media-rail::-webkit-scrollbar {
        display: none;
      }

      .output-media-slide {
        position: relative;
        flex: 0 0 100%;
        min-width: 0;
        height: 100%;
        margin: 0;
        padding: 52px 54px 22px;
        scroll-snap-align: start;
        scroll-snap-stop: always;
      }

      .output-media-hero[data-count="1"] .output-media-slide {
        padding-inline: 12px;
      }

      .output-media-slide > img {
        display: block;
        width: 100%;
        height: 100%;
        object-fit: contain;
        filter: drop-shadow(0 10px 18px color-mix(in srgb, CanvasText 15%, transparent));
      }

      .output-media-slide.output-media-html-slide,
      .output-media-hero[data-count="1"] .output-media-html-slide {
        padding: 0;
        contain: layout paint;
        background: Canvas;
        color: CanvasText;
      }

      .output-html-frame {
        display: block;
        border: 0;
        width: 100%;
        height: 100%;
        min-height: 0;
      }

      .output-html-expanded-header {
        display: none;
      }

      .output-media-html-slide:fullscreen {
        display: flex;
        flex-direction: column;
        width: 100%;
        height: 100%;
      }

      .output-media-html-slide:fullscreen > .output-html-expanded-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 10px 12px;
        font-size: 12px;
        flex: 0 0 auto;
      }

      .output-media-html-slide:fullscreen > .output-html-frame {
        flex: 1 1 auto;
      }

      .output-media-slide[data-load-state="failed"] > img {
        visibility: hidden;
      }

      .output-media-state {
        position: absolute;
        inset: 52px 54px 22px;
        display: grid;
        place-content: center;
        margin: 0;
        color: CanvasText;
        text-align: center;
        font-size: 12px;
      }

      .output-media-overlay {
        position: absolute;
        inset: 10px 12px auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 10px;
        pointer-events: none;
      }

      .output-media-overlay > * {
        pointer-events: auto;
      }

      .output-media-counter {
        padding: 5px 10px;
        border-radius: 999px;
        background: Canvas;
        color: CanvasText;
        font-size: 12px;
        font-variant-numeric: tabular-nums;
      }

      .output-media-tools {
        display: flex;
        gap: 6px;
        margin-inline-start: auto;
      }

      .output-media-tool,
      .output-media-arrow {
        appearance: none;
        display: grid;
        place-items: center;
        min-width: 32px;
        width: 32px;
        height: 32px;
        margin: 0;
        padding: 0;
        border: 1px solid color-mix(in srgb, CanvasText 12%, Canvas);
        border-radius: 50%;
        background: Canvas;
        color: CanvasText;
        font: inherit;
        font-size: 14px;
        line-height: 1;
      }

      .output-media-tool:hover:not(:disabled),
      .output-media-arrow:hover:not(:disabled),
      .output-media-tool[aria-expanded="true"] {
        background: SelectedItem;
        color: SelectedItemText;
      }

      .output-media-tool:disabled,
      .output-media-arrow:disabled {
        opacity: 0.4;
        cursor: default;
      }

      .output-media-tool:focus-visible,
      .output-media-arrow:focus-visible,
      .output-media-explanation:focus-visible,
      .output-media-return:focus-visible {
        outline: 2px solid AccentColor;
        outline-offset: -2px;
      }

      .output-media-arrow {
        position: absolute;
        top: 50%;
        width: 44px;
        height: 58px;
        transform: translateY(-50%);
        border: 0;
        font-size: 18px;
        box-shadow: 0 4px 16px color-mix(in srgb, CanvasText 12%, transparent);
      }

      .output-media-previous {
        left: 0;
        border-radius: 0 999px 999px 0;
      }

      .output-media-next {
        right: 0;
        border-radius: 999px 0 0 999px;
      }

      .output-media-details {
        position: absolute;
        z-index: 2;
        top: 50px;
        right: 12px;
        width: min(300px, calc(100% - 24px));
        max-height: calc(100% - 62px);
        overflow: auto;
        padding: 15px;
        border: 1px solid ButtonBorder;
        border-radius: 12px;
        background: Canvas;
        color: CanvasText;
        box-shadow: 0 10px 28px color-mix(in srgb, CanvasText 16%, transparent);
      }

      .output-media-details[hidden] {
        display: none;
      }

      .output-media-details h2 {
        margin: 0 0 12px;
        font-size: 14px;
      }

      .output-media-details dl {
        display: grid;
        grid-template-columns: auto minmax(0, 1fr);
        gap: 10px 14px;
        margin: 0;
        font-size: 12px;
      }

      .output-media-details dt {
        color: GrayText;
      }

      .output-media-details dd {
        margin: 0;
        text-align: right;
        overflow-wrap: anywhere;
      }

      .output-media-explanation,
      .output-media-return {
        appearance: none;
        border: 0;
        background: transparent;
        color: CanvasText;
        font: inherit;
        font-size: 12px;
      }

      .output-media-explanation {
        display: block;
        width: 100%;
        height: var(--output-media-cue-height);
        padding: 4px 12px;
        border-bottom: 1px solid Separator;
        border-radius: 0;
      }

      .output-media-return {
        align-self: flex-start;
        padding: 0;
        color: GrayText;
      }

      .output-media-explanation .fa-solid,
      .output-media-return .fa-solid {
        margin-inline: 3px;
        font-size: 11px;
      }

      .output-media-expanded {
        display: none;
        position: fixed;
        inset: 0;
        width: 100%;
        height: 100%;
        max-width: none;
        max-height: none;
        margin: 0;
        padding: 0 12px 12px;
        border: 0;
        background: Canvas;
        color: CanvasText;
      }

      .output-media-expanded:fullscreen {
        display: flex;
        flex-direction: column;
      }

      .output-media-expanded::backdrop {
        background: Canvas;
      }

      .output-media-expanded > header {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        padding-block: 10px;
        font-size: 12px;
      }

      .output-media-expanded > img {
        flex: 1 1 auto;
        min-height: 0;
        width: 100%;
        object-fit: contain;
      }

      @media (pointer: coarse) {
        .output-media-tool {
          width: 44px;
          height: 44px;
        }
        .output-media-slide {
          padding-top: 64px;
        }
        .output-media-details {
          top: 64px;
          max-height: calc(100% - 76px);
        }
        .overlay-shell {
          --output-media-cue-size: 44px;
        }
      }

      @media (prefers-contrast: more), (prefers-reduced-transparency: reduce) {
        .output-media-ambient {
          display: none;
        }
        .output-media-tool,
        .output-media-arrow {
          background: ButtonFace;
          color: ButtonText;
          border: 1px solid ButtonText;
          box-shadow: none;
        }
      }

      .lens-output-unsupported {
        margin: 0;
        padding: 12px;
        border: 1px solid Separator;
        border-radius: 7px;
        color: GrayText;
        background: color-mix(in srgb, CanvasText 4%, transparent);
      }

      .markdown-body {
        display: block;
        overflow-wrap: anywhere;
      }

      .markdown-body > :first-child {
        margin-top: 0;
      }

      .markdown-body > :last-child {
        margin-bottom: 0;
      }

      .markdown-body h1,
      .markdown-body h2,
      .markdown-body h3,
      .markdown-body h4,
      .markdown-body h5,
      .markdown-body h6 {
        margin: 1.35em 0 0.55em;
        line-height: 1.25;
        font-weight: 650;
      }

      .markdown-body h1 {
        padding-bottom: 0.3em;
        border-bottom: 1px solid Separator;
        font-size: 1.7em;
      }

      .markdown-body h2 {
        padding-bottom: 0.3em;
        border-bottom: 1px solid Separator;
        font-size: 1.4em;
      }

      .markdown-body h3 {
        font-size: 1.2em;
      }

      .markdown-body h4,
      .markdown-body h5,
      .markdown-body h6 {
        font-size: 1em;
      }

      .markdown-body p,
      .markdown-body blockquote,
      .markdown-body ul,
      .markdown-body ol,
      .markdown-body pre,
      .markdown-body table {
        margin: 0 0 1em;
      }

      .markdown-body ul,
      .markdown-body ol {
        padding-left: 2em;
      }

      .markdown-body li + li {
        margin-top: 0.25em;
      }

      .markdown-body li > p {
        margin: 0.25em 0;
      }

      .markdown-body blockquote {
        padding: 0 1em;
        border-left: 3px solid Separator;
        color: GrayText;
      }

      .markdown-body code,
      .markdown-body kbd,
      .markdown-body pre {
        font-family: ui-monospace, "SFMono-Regular", Menlo, Monaco, monospace;
        font-size: 0.92em;
      }

      .markdown-body code,
      .markdown-body kbd {
        border-radius: 4px;
        padding: 0.15em 0.35em;
        background: color-mix(in srgb, CanvasText 8%, transparent);
      }

      .markdown-body pre {
        max-width: 100%;
        overflow: auto;
        border: 1px solid Separator;
        border-radius: 7px;
        padding: 12px;
        background: color-mix(in srgb, CanvasText 6%, transparent);
        white-space: pre;
      }

      .markdown-body pre code {
        padding: 0;
        background: transparent;
      }

      .markdown-body table {
        display: block;
        max-width: 100%;
        overflow: auto;
        border-collapse: collapse;
      }

      .markdown-body th,
      .markdown-body td {
        padding: 6px 10px;
        border: 1px solid Separator;
      }

      .markdown-body th {
        font-weight: 650;
        background: color-mix(in srgb, CanvasText 5%, transparent);
      }

      .markdown-body tr:nth-child(even) {
        background: color-mix(in srgb, CanvasText 3%, transparent);
      }

      .markdown-body hr {
        height: 1px;
        margin: 1.5em 0;
        border: 0;
        background: Separator;
      }

      .markdown-body a {
        color: LinkText;
        text-decoration-thickness: from-font;
      }

      .markdown-body img {
        max-width: 100%;
        height: auto;
      }

      .markdown-body input[type="checkbox"] {
        margin: 0 0.45em 0.25em;
        accent-color: AccentColor;
      }

      .markdown-body .mermaid-diagram {
        max-width: 100%;
        margin: 0 0 1em;
        overflow: auto;
        border: 1px solid Separator;
        border-radius: 7px;
        padding: 12px;
        background: color-mix(in srgb, Canvas 94%, transparent);
      }

      .markdown-body .mermaid-diagram svg {
        display: block;
        max-width: 100%;
        height: auto;
        margin: auto;
      }

      .markdown-body .mermaid-error-message {
        margin: -0.6em 0 1em;
        color: CanvasText;
        font-size: 0.9em;
        opacity: 0.72;
      }

      .markdown-body .streaming-cursor {
        display: inline-block;
        margin-inline-start: 0.1em;
        color: AccentColor;
        font-weight: 500;
      }

      .markdown-body .ce-cursor-blink {
        animation: streaming-cursor-blink 1s steps(1, end) infinite;
      }

      @keyframes streaming-cursor-blink {
        0%,
        52% {
          opacity: 1;
        }
        53%,
        100% {
          opacity: 0;
        }
      }

      .empty-state {
        flex: 1 1 auto;
        min-height: 0;
        margin: 0;
        padding: 28px;
        display: flex;
        align-items: center;
        justify-content: center;
        text-align: center;
      }

      .empty-state {
        color: GrayText;
      }

      .overlay-main > .error,
      .overlay-main > .notice {
        margin: 8px 14px;
        padding: 8px;
        border-radius: 5px;
      }

      .notice {
        color: CanvasText;
        background: SelectedItem;
      }

      .overlay-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        justify-content: flex-end;
        gap: 8px;
        padding: 8px 14px;
        border-block: 1px solid Separator;
      }

      .overlay-actions p {
        margin: 0 auto 0 0;
        color: GrayText;
      }

      .extraction-diagnostics {
        flex: 1 1 auto;
        min-height: 0;
        overflow: auto;
        margin: 10px 14px 16px;
        border: 1px solid Separator;
        border-radius: 10px;
        padding: 10px;
        color: GrayText;
        font-size: 11px;
        background: color-mix(in srgb, CanvasText 3%, transparent);
      }

      .diagnostics-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        margin-bottom: 10px;
        border: 1px solid Separator;
        border-radius: 7px;
        padding: 7px 9px;
        color: CanvasText;
        background: color-mix(in srgb, Canvas 78%, transparent);
      }

      .diagnostics-header h2 {
        min-width: 0;
        margin: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-size: 11px;
        font-weight: 650;
      }

      .diagnostic-count {
        flex: 0 0 auto;
        border: 1px solid Separator;
        border-radius: 999px;
        padding: 1px 7px;
        color: GrayText;
        font-size: 10px;
        font-weight: 400;
      }

      .diagnostics-layout {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(min(190px, 100%), 1fr));
        gap: 12px;
      }

      .diagnostic-group {
        min-width: 0;
      }

      .diagnostic-group h2 {
        margin: 0 0 6px;
        color: GrayText;
        font-size: 10px;
        font-variant-caps: all-small-caps;
        letter-spacing: 0.04em;
      }

      .diagnostic-messages {
        grid-column: 1 / -1;
      }

      .diagnostic-messages p,
      .diagnostic-messages ul {
        margin: 0;
      }

      .diagnostic-messages ul {
        padding-inline-start: 18px;
      }

      .diagnostic-messages li {
        overflow-wrap: anywhere;
      }

      .diagnostic-messages li + li {
        margin-top: 4px;
      }

      .metrics {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(min(112px, 100%), 1fr));
        gap: 6px;
        margin: 0;
      }

      .metrics > div {
        min-width: 0;
        border: 1px solid Separator;
        border-radius: 6px;
        padding: 6px 8px;
        background: color-mix(in srgb, Canvas 75%, transparent);
      }

      .metrics dt {
        margin: 0 0 2px;
        font-size: 10px;
      }

      .metrics dd {
        margin: 0;
        overflow-wrap: anywhere;
        color: CanvasText;
        font-variant-numeric: tabular-nums;
      }

      @media (prefers-reduced-transparency: reduce), (prefers-contrast: more) {
        .overlay-shell {
          border-width: 2px;
          background: Canvas;
        }

        .lens-progress-snackbar {
          background: Canvas;
          -webkit-backdrop-filter: none;
          backdrop-filter: none;
        }
      }

      lens-session-controls {
        display: block;
        min-height: 0;
        overflow: auto;
        overscroll-behavior: contain;
      }

      .session-controls {
        padding: 0 14px 8px;
        border-bottom: 1px solid Separator;
      }

      .session-controls label,
      .session-form-field {
        display: grid;
        gap: 4px;
        margin-block: 8px;
      }

      .session-controls select,
      .session-controls input {
        max-width: 100%;
        min-width: 0;
      }

      .session-controls pre,
      .elicitation-url {
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        max-height: 160px;
        overflow: auto;
      }

      .session-controls button {
        margin-inline-end: 6px;
        margin-block: 4px;
      }
    `,
    feedbackStyles,
    accessibilityStyles,
    reducedMotionStyles,
    ...sharedIconStyles,
  ];

  @property({ type: Boolean }) active = initialOverlayState().active;
  @property({ attribute: false }) htmlContent: HtmlOutputContent | undefined;

  @property({ attribute: false })
  model: OverlayViewModel | undefined = initialOverlayState().model;
  @property({ attribute: false }) snapshotStatus = initialOverlayState().snapshotStatus;

  @state()
  private activeTab: LensTab = initialOverlayState().activeTab;

  @state()
  private displayedRepresentation: LensRepresentation | undefined;

  private synchronizedOperationId: string | undefined;
  private hasSynchronizedOperation = false;
  private pendingScrollPosition: InterpretationScrollPosition | undefined;
  private restoreInterpretationFocus = false;
  private notificationIdentity: string | undefined;

  @state()
  private notificationVisibility: "open" | "closed" = initialOverlayState().notificationVisibility;

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("model")) return;
    const previous = changed.get("model");
    if (previous?.lens.operation_id !== this.model?.lens.operation_id) {
      this.activeTab = "interpretation";
    }
    if (this.model) this.synchronizeRepresentation(this.model.lens);
    const notification = this.model
      ? overlayNotification(this.lensWithDisplayedRepresentation(this.model.lens))
      : undefined;
    const identity = notification
      ? JSON.stringify([
          this.model?.lens.operation_id,
          notification.title,
          this.model?.lens.agent?.run_id,
          this.model?.lens.stage === "transforming" ? undefined : notification.detail,
          notification.busy,
          notification.prominent,
        ])
      : undefined;
    if (identity !== this.notificationIdentity) {
      this.notificationIdentity = identity;
      this.notificationVisibility = notification?.prominent ? "open" : "closed";
    }
  }

  protected render() {
    const model = this.model;
    const lens = model?.lens;
    const context = lens?.context;
    const targets = lens?.target_set?.targets ?? [];
    const sourceJson = lens ? lensSourceJson(lens) : "";
    const activeAgent = lens?.agent;
    const authenticationMethods = lens ? supportedAuthMethods(lens) : [];
    const targetLabels = targets.map(({ facts }) =>
      facts.title ? `${facts.application_name} — ${facts.title}` : facts.application_name,
    );
    const sourceCountLabel = !lens
      ? nothing
      : targets.length === 0
        ? "No selected windows"
        : `${targets.length} selected ${targets.length === 1 ? "window" : "windows"}`;
    const sourceContext = !lens
      ? nothing
      : targetLabels.length
        ? targetLabels.join(" · ")
        : "No source context is available.";
    const sourceContextTitle = !lens
      ? ""
      : targetLabels.length
        ? targetLabels.join("\n")
        : "No source context is available.";
    const canCancel = lens?.stage === "connecting" || lens?.stage === "transforming";
    const canRetry =
      Boolean(lens?.input) &&
      (lens?.stage === "authentication_required" || lens?.stage === "failed");
    const liveStatus = lensLiveStatus(lens?.live);
    const displayLens = lens ? this.lensWithDisplayedRepresentation(lens) : undefined;
    const announcedStatus = displayLens ? overlayNotification(displayLens) : undefined;
    const interactive = Boolean(
      lens?.session_controls?.active &&
      lens?.session_controls.interactions.some((i) => i.status === "pending"),
    );
    const showStatusSnackbar = Boolean(
      announcedStatus && (interactive || this.notificationVisibility === "open"),
    );
    const persistentStatus = liveStatus;
    const outputMedia = displayLens
      ? composeOutputMedia(lensOutputPresentation(displayLens))
      : { media: [], narrative: [] };
    const hasMediaCue =
      this.activeTab === "interpretation" &&
      outputMedia.media.length > 0 &&
      outputMedia.narrative.length > 0;

    return html`
      <div
        class="overlay-shell"
        data-progress=${showStatusSnackbar ? "true" : "false"}
        data-media-cue=${hasMediaCue ? "true" : "false"}
        @lens-agent-output-intent=${this.forwardOutputIntent}
      >
        <header class="overlay-header" data-tauri-drag-region="deep">
          <div class="overlay-brand">
            <img class="overlay-app-icon" src=${appIconUrl} alt="" />
            <h1 class="overlay-title visually-hidden">Lens</h1>
          </div>
          <div class="overlay-header-actions">
            ${
              canCancel
                ? html`<button
                    type="button"
                    class="overlay-header-action"
                    data-tauri-drag-region="false"
                    ?disabled=${model?.cancelPending}
                    @click=${() => this.emit({ type: "cancel" })}
                  >
                    Cancel
                  </button>`
                : nothing
            }
            ${
              canRetry
                ? html`<button
                    type="button"
                    class="overlay-header-action"
                    data-tauri-drag-region="false"
                    ?disabled=${model?.pending}
                    @click=${() => this.emit({ type: "retry" })}
                  >
                    Retry with Agent
                  </button>`
                : nothing
            }
            ${
              lens?.live?.lifecycle === "watching"
                ? html`<button
                    type="button"
                    class="overlay-header-action"
                    data-tauri-drag-region="false"
                    @click=${() => this.emit({ type: "pause" })}
                  >
                    Pause Updates
                  </button>`
                : lens?.live?.lifecycle === "paused"
                  ? html`<button
                      type="button"
                      class="overlay-header-action"
                      data-tauri-drag-region="false"
                      @click=${() => this.emit({ type: "resume" })}
                    >
                      Resume Updates
                    </button>`
                  : nothing
            }
            <button
              type="button"
              class="close-button"
              ?disabled=${!this.active}
              data-tauri-drag-region="false"
              aria-label=${lens?.operation_id ? "Stop Lens and close" : "Close Lens"}
              title=${lens?.operation_id ? "Stop Lens and close" : "Close Lens"}
              @click=${() => this.emit({ type: "close" })}
            >
              <i class="fa-solid fa-xmark" aria-hidden="true"></i>
            </button>
          </div>
        </header>

        <section class="overlay-source-summary" aria-label="Selected source context">
          <span class="overlay-source-icon" aria-hidden="true">
            <i class="fa-solid fa-window-maximize"></i>
          </span>
          <div class="overlay-source-copy">
            <strong class="overlay-source-count">${sourceCountLabel}</strong>
            <span class="overlay-source-targets" title=${sourceContextTitle}>${sourceContext}</span>
          </div>
        </section>

        <nav class="lens-tabs" aria-label="Lens content">
          <div role="tablist" aria-orientation="horizontal">
            ${LENS_TABS.map(({ id, label }) => this.renderTab(id, label))}
          </div>
        </nav>

        <main class="overlay-main">
          ${renderSnapshotFailure(this.snapshotStatus)}
          <div data-region-error="output"></div>
          <div data-region-error="session"></div>
          <div data-region-error="source"></div>
          ${model?.message ? html`<p class="error" role="alert">${model?.message}</p>` : nothing}
          ${lens?.error ? html`<p class="error" role="alert">${lens?.error}</p>` : nothing}
          ${
            activeAgent?.authentication_message
              ? html`<p class="notice" role="status">${activeAgent.authentication_message}</p>`
              : nothing
          }
          ${
            lens?.stage === "authentication_required"
              ? html`<section class="overlay-actions" aria-label="Agent authentication">
                  ${
                    authenticationMethods.length
                      ? authenticationMethods.map(
                          (method) => html`<button
                            ?disabled=${model?.pending}
                            @click=${() => this.emit({ type: "authenticate", methodId: method.id })}
                          >
                            Authenticate with ${method.name}…
                          </button>`,
                        )
                      : html`<p>Authenticate with this agent's existing CLI, then try again.</p>`
                  }
                </section>`
              : nothing
          }
          ${this.renderActivePanel(lens, displayLens, sourceJson)}
        </main>

        <div id="lens-progress-notification" class="lens-progress-region">
          ${
            announcedStatus
              ? html`<div
                  class=${showStatusSnackbar ? "lens-progress-snackbar" : "visually-hidden"}
                  data-interactive=${interactive}
                >
                  <div
                    class="lens-status-announcement"
                    role="status"
                    aria-live="polite"
                    aria-atomic="true"
                  >
                    <i
                      class=${
                        announcedStatus.busy
                          ? "fa-solid fa-spinner fa-spin"
                          : "fa-solid fa-circle-info lens-status-icon"
                      }
                      aria-hidden="true"
                    ></i>
                    <span class="lens-progress-copy">
                      <strong>${announcedStatus.title}</strong>
                      ${interactive ? nothing : html`<span aria-hidden=${lens?.stage === "transforming" ? "true" : "false"}>${announcedStatus.detail}</span>`}
                    </span>
                  </div>
                  ${
                    showStatusSnackbar && !interactive
                      ? html`<button
                          type="button"
                          class="close-button lens-progress-dismiss"
                          aria-label="Dismiss notification"
                          title="Dismiss notification"
                          @click=${this.dismissNotification}
                        >
                          <i class="fa-solid fa-xmark" aria-hidden="true"></i>
                        </button>`
                      : nothing
                  }
                  ${
                    interactive
                      ? html`<lens-session-controls
                          presentation="interaction"
                          .controls=${lens?.session_controls}
                          .submission=${model?.interactionSubmission}
                        ></lens-session-controls>`
                      : nothing
                  }
                </div>`
              : nothing
          }
        </div>

        <footer class="overlay-footer">
          ${
            announcedStatus && !interactive
              ? html`<button
                  type="button"
                  class="overlay-footer-status overlay-status-toggle"
                  aria-controls="lens-progress-notification"
                  aria-expanded=${showStatusSnackbar ? "true" : "false"}
                  aria-label="${showStatusSnackbar ? "Hide" : "Show"} status details: ${announcedStatus.title}"
                  title=${showStatusSnackbar ? "Hide status details" : "Show status details"}
                  @click=${this.toggleNotification}
                >
                  <span class="overlay-stage-indicator" aria-hidden="true"></span>
                  <span class="overlay-stage">${announcedStatus.title}</span>
                </button>`
              : html`<div
                  class="overlay-footer-status"
                  title=${persistentStatus?.detail ?? (lens ? STAGE_LABEL[lens.stage] : nothing)}
                >
                  <span class="overlay-stage-indicator" aria-hidden="true"></span>
                  <span class="overlay-stage"
                    >${interactive ? "Agent response required" : (persistentStatus?.title ?? (lens ? STAGE_LABEL[lens.stage] : nothing))}</span
                  >
                </div>`
          }
          ${
            context
              ? html`<span class="quality quality-${context.quality}">${context.quality}</span>`
              : nothing
          }
        </footer>
      </div>
    `;
  }

  private renderActivePanel(
    lens: OverlayViewModel["lens"] | undefined,
    displayLens: LensState | undefined,
    sourceJson: string,
  ) {
    const activeTab = this.activeTab;
    if (!lens || !displayLens)
      return html`<section
        id="${activeTab}-panel"
        class="lens-panel"
        role="tabpanel"
        aria-labelledby="${activeTab}-tab"
        aria-busy="true"
        tabindex="0"
      ></section>`;
    switch (activeTab) {
      case "interpretation":
        return html`<section
          id="interpretation-panel"
          class="lens-panel"
          role="tabpanel"
          aria-labelledby="interpretation-tab"
          tabindex="0"
        >
          <lens-agent-output
            .lens=${displayLens}
            .htmlContent=${this.htmlContent}
          ></lens-agent-output>
        </section>`;
      case "source":
        return html`<section
          id="source-panel"
          class="lens-panel"
          role="tabpanel"
          aria-labelledby="source-tab"
          tabindex="0"
        >
          ${
            sourceJson
              ? html`<div class="lens-content source-view">
                  <lens-media-gallery .lens=${lens}></lens-media-gallery>
                  <section class="source-json" aria-labelledby="source-json-heading">
                    <h2 id="source-json-heading">Structured input</h2>
                    <pre
                      class="source-content"
                      aria-label="Normalized Lens source JSON"
                    ><code>${sourceJson}</code></pre>
                  </section>
                </div>`
              : html`<div class="lens-content">
                  <p class="empty-state">No normalized source data is available.</p>
                </div>`
          }
        </section>`;
      case "diagnostics":
        return html`<section
          id="diagnostics-panel"
          class="lens-panel"
          role="tabpanel"
          aria-labelledby="diagnostics-tab"
          tabindex="0"
        >
          <lens-session-controls .controls=${lens.session_controls}></lens-session-controls>
          ${
            lens.context
              ? html`<lens-extraction-diagnostics
                  .context=${lens.context}
                  .agent=${lens.agent}
                ></lens-extraction-diagnostics>`
              : html`<div class="lens-content">
                  <p class="empty-state">No extraction diagnostics are available.</p>
                </div>`
          }
        </section>`;
    }
  }

  private renderTab(tab: LensTab, label: string) {
    const selected = this.activeTab === tab;
    return html`<button
      type="button"
      id="${tab}-tab"
      class="lens-tab"
      ?disabled=${!this.active}
      role="tab"
      aria-selected=${selected ? "true" : "false"}
      aria-controls="${tab}-panel"
      tabindex=${selected ? 0 : -1}
      @click=${() => this.activateTab(tab)}
      @keydown=${this.handleTabKeyDown}
    >
      ${label}
    </button>`;
  }

  private handleTabKeyDown = (event: KeyboardEvent): void => {
    const activeIndex = LENS_TABS.findIndex(({ id }) => id === this.activeTab);
    const nextIndex = (() => {
      switch (event.key) {
        case "ArrowLeft":
          return (activeIndex - 1 + LENS_TABS.length) % LENS_TABS.length;
        case "ArrowRight":
          return (activeIndex + 1) % LENS_TABS.length;
        case "Home":
          return 0;
        case "End":
          return LENS_TABS.length - 1;
        default:
          return undefined;
      }
    })();
    if (nextIndex === undefined) return;
    const nextTab = LENS_TABS[nextIndex]?.id;
    if (!nextTab) return;
    event.preventDefault();
    this.activateTab(nextTab);
    void this.updateComplete.then(() => {
      this.renderRoot.querySelector<HTMLButtonElement>(`#${nextTab}-tab`)?.focus();
    });
  };

  private forwardOutputIntent = (event: CustomEvent<AgentOutputIntent>): void => {
    event.stopPropagation();
    this.emit(event.detail);
  };

  private emit(intent: OverlayIntent): void {
    dispatchComponentEvent(this, OVERLAY_INTENT_EVENT, intent);
  }

  private dismissNotification = (): void => {
    this.notificationVisibility = "closed";
    void this.updateComplete.then(() => {
      this.renderRoot
        .querySelector<HTMLButtonElement>(".overlay-status-toggle")
        ?.focus({ preventScroll: true });
    });
  };

  private toggleNotification = (): void => {
    this.notificationVisibility = this.notificationVisibility === "open" ? "closed" : "open";
  };

  private activateTab(tab: LensTab): void {
    this.activeTab = tab;
  }

  private synchronizeRepresentation(lens: LensState): void {
    const operationId = lens.operation_id;
    const representation = lens.representation;
    if (!this.hasSynchronizedOperation || operationId !== this.synchronizedOperationId) {
      this.hasSynchronizedOperation = true;
      this.synchronizedOperationId = operationId;
      this.displayedRepresentation = representation;
      return;
    }
    if (!representation) return;
    if (representation.representation_id === this.displayedRepresentation?.representation_id) {
      return;
    }
    this.acceptRepresentation(representation, this.interpretationHasFocus());
  }

  private lensWithDisplayedRepresentation(lens: LensState): LensState {
    const representation = this.displayedRepresentation;
    if (!representation || representation === lens.representation) return lens;
    return { ...lens, representation };
  }

  private interpretationHasFocus(): boolean {
    const panel = this.renderRoot.querySelector<HTMLElement>("#interpretation-panel");
    if (!panel) return false;
    const activeElement = this.shadowRoot?.activeElement;
    return Boolean(activeElement && panel.contains(activeElement));
  }

  private acceptRepresentation(
    representation: LensRepresentation,
    restoreInterpretationFocus = false,
  ): void {
    if (representation.representation_id === this.displayedRepresentation?.representation_id) {
      return;
    }
    this.pendingScrollPosition = this.captureInterpretationScrollPosition();
    this.restoreInterpretationFocus ||= restoreInterpretationFocus;
    this.displayedRepresentation = representation;
    void this.updateComplete.then(() => this.restoreInterpretationPresentation());
  }

  private captureInterpretationScrollPosition(): InterpretationScrollPosition | undefined {
    const output = this.renderRoot
      .querySelector("lens-agent-output")
      ?.querySelector<HTMLElement>(".lens-output");
    if (!output) return undefined;
    const maximum = Math.max(0, output.scrollHeight - output.clientHeight);
    return {
      top: output.scrollTop,
      wasAtBottom: maximum - output.scrollTop <= 36,
      hadMedia: output.classList.contains("has-media"),
    };
  }

  private async restoreInterpretationPresentation(): Promise<void> {
    const outputComponent = this.renderRoot.querySelector<
      HTMLElement & {
        updateComplete: Promise<boolean>;
      }
    >("lens-agent-output");
    await outputComponent?.updateComplete;
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    const output = outputComponent?.querySelector<HTMLElement>(".lens-output");
    const position = this.pendingScrollPosition;
    this.pendingScrollPosition = undefined;
    if (output && position) {
      const maximum = Math.max(0, output.scrollHeight - output.clientHeight);
      output.scrollTop = output.classList.contains("has-media")
        ? position.hadMedia
          ? Math.min(position.top, maximum)
          : 0
        : position.wasAtBottom
          ? maximum
          : Math.min(position.top, maximum);
    }
    if (this.restoreInterpretationFocus) {
      this.restoreInterpretationFocus = false;
      this.renderRoot
        .querySelector<HTMLElement>("#interpretation-panel")
        ?.focus({ preventScroll: true });
    }
  }
}
