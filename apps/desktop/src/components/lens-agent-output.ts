import { LitElement, html, nothing, type PropertyValues } from "lit";
import { composeOutputMedia } from "../output-media";
import type { HtmlOutputContent } from "../application/html-output-controller";
import "./lens-output-media";
import { customElement, property } from "lit/decorators.js";
import { externalMarkdownUrl } from "../markdown";
import "../streaming-markdown";
import type { StreamingMarkdownState } from "../streaming-markdown";
import type { LensOutputBlock, LensState } from "../types";
import { imageDataUrl, lensOutputPresentation, type LensOutputMode } from "../view-model";
import {
  AGENT_OUTPUT_INTENT_EVENT,
  dispatchComponentEvent,
  type AgentOutputIntent,
} from "./events";

@customElement("lens-agent-output")
export class LensAgentOutput extends LitElement {
  @property({ attribute: false })
  lens: LensState = { stage: "idle", output_blocks: [] };
  @property({ attribute: false }) htmlContent: HtmlOutputContent | undefined;

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  private revealFirstMedia = false;

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("lens")) return;
    const previous = changed.get("lens");
    const hasMedia = composeOutputMedia(lensOutputPresentation(this.lens)).media.length > 0;
    const hadMedia = previous
      ? composeOutputMedia(lensOutputPresentation(previous)).media.length > 0
      : false;
    this.revealFirstMedia =
      hasMedia && (!hadMedia || previous?.operation_id !== this.lens.operation_id);
  }

  protected updated(): void {
    if (!this.revealFirstMedia) return;
    this.revealFirstMedia = false;
    const output = this.querySelector<HTMLElement>(".lens-output");
    if (output) output.scrollTop = 0;
  }

  protected render() {
    const output = lensOutputPresentation(this.lens);
    const { media, narrative } = composeOutputMedia(output);
    if (output.blocks.length) {
      return html`<div
        class="lens-content lens-output ${media.length ? "has-media" : ""} ${narrative.length ? "has-narrative" : ""}"
        data-auto-scroll-container
        role="document"
      >
        ${media.length ? html`<lens-output-media .media=${media} .htmlContent=${this.htmlContent}></lens-output-media>` : nothing}
        ${
          media.length && narrative.length
            ? html`
                <button
                  type="button"
                  class="output-media-explanation"
                  @click=${this.showExplanation}
                >
                  Explore the interpretation
                  <i class="fa-solid fa-arrow-down" aria-hidden="true"></i>
                </button>
              `
            : nothing
        }
        ${
          narrative.length
            ? html`<div class="lens-output-narrative">
                ${
                  media.length
                    ? html`<button
                        type="button"
                        class="output-media-return"
                        @click=${this.showMedia}
                      >
                        <i class="fa-solid fa-arrow-up" aria-hidden="true"></i> Back to media
                      </button>`
                    : nothing
                }
                ${narrative.map(({ block, index }) =>
                  this.renderBlock(
                    block,
                    index,
                    output.identity,
                    output.mode,
                    index === output.blocks.length - 1,
                    media.length > 0,
                  ),
                )}
              </div>`
            : nothing
        }
      </div>`;
    }
    return this.renderEmpty();
  }

  private showExplanation = (): void => {
    const output = this.querySelector<HTMLElement>(".lens-output");
    const narrative = this.querySelector<HTMLElement>(".lens-output-narrative");
    if (!output || !narrative) return;
    output.scrollTo({
      top:
        narrative.getBoundingClientRect().top -
        output.getBoundingClientRect().top +
        output.scrollTop,
      behavior: this.scrollBehavior(),
    });
    this.querySelector<HTMLButtonElement>(".output-media-return")?.focus({ preventScroll: true });
  };

  private showMedia = (): void => {
    this.querySelector<HTMLElement>(".lens-output")?.scrollTo({
      top: 0,
      behavior: this.scrollBehavior(),
    });
    this.querySelector<HTMLButtonElement>(".output-media-details-toggle")?.focus({
      preventScroll: true,
    });
  };

  private scrollBehavior(): ScrollBehavior {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth";
  }

  private renderBlock(
    block: LensOutputBlock,
    index: number,
    identity: string | undefined,
    mode: LensOutputMode,
    isLastBlock: boolean,
    hasMedia: boolean,
  ) {
    switch (block.type) {
      case "markdown":
        return html`<lens-markdown
          class="markdown-body"
          .state=${
            {
              operationId: identity ? `${identity}:${index}` : undefined,
              markdown: block.text,
              phase: mode === "initial-stream" && isLastBlock ? "streaming" : "settled",
              scrollBehavior: hasMedia ? "preserve" : "follow",
            } satisfies StreamingMarkdownState
          }
          @click=${this.openMarkdownLink}
          @markdown-render-error=${this.handleMarkdownRenderError}
        ></lens-markdown>`;
      case "image": {
        const source = imageDataUrl(block);
        return source
          ? html`<figure class="lens-output-image">
              <img src=${source} alt="Visual output from the agent" />
            </figure>`
          : this.renderUnsupported(`image (${block.mime_type})`);
      }
      case "unsupported":
        return this.renderUnsupported(block.content_type);
      case "html":
        return nothing;
    }
  }

  private renderUnsupported(contentType: string) {
    return html`<p class="lens-output-unsupported" role="note">
      This agent output type is not supported yet: <code>${contentType}</code>
    </p>`;
  }

  private renderEmpty() {
    const message = (() => {
      switch (this.lens.stage) {
        case "idle":
          return "Select one or more Lens Targets from the menu bar.";
        case "authentication_required":
          return "Authenticate the selected Agent to continue.";
        case "cancelled":
          return "The Agent transformation was cancelled.";
        case "failed":
          return "The Agent did not produce an interpretation.";
        case "completed":
          return "The Agent completed without returning displayable content.";
        case "selecting":
        case "extracting":
        case "connecting":
        case "transforming":
          return "The Agent's interpretation will appear here.";
        case "ready":
          return "Transform the selected content with the configured Agent.";
      }
    })();
    return html`<p class="empty-state">${message}</p>`;
  }

  private openMarkdownLink = (event: MouseEvent): void => {
    const link = event
      .composedPath()
      .find((candidate): candidate is HTMLAnchorElement => candidate instanceof HTMLAnchorElement);
    if (!link) return;
    event.preventDefault();
    const externalUrl = externalMarkdownUrl(link.getAttribute("href") ?? "");
    this.emit(
      externalUrl
        ? { type: "open-external-url", url: externalUrl }
        : { type: "report-error", message: "This Markdown link uses an unsupported URL." },
    );
  };

  private handleMarkdownRenderError = (event: CustomEvent<string>): void => {
    event.stopPropagation();
    this.emit({ type: "report-error", message: `Unable to render Markdown: ${event.detail}` });
  };

  private emit(intent: AgentOutputIntent): void {
    dispatchComponentEvent(this, AGENT_OUTPUT_INTENT_EVENT, intent);
  }
}
