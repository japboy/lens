import { LitElement, html, nothing, type PropertyValues } from "lit";
import { repeat } from "lit/directives/repeat.js";
import { keyed } from "lit/directives/keyed.js";
import type {
  ResponseHistoryPresentation,
  LoadResponseBlock,
} from "../application/response-history-controller";
import type { LensResponseBlock } from "./lens-response-block";
import type { LensOutputMedia } from "./lens-output-media";
import { responseBlockIdentity } from "../application/response-history-controller";
import "./lens-response-block";
import { composeOutputMedia } from "../output-media";
import type { HtmlOutputContent } from "../application/html-output-controller";
import "./lens-output-media";
import { customElement, property } from "lit/decorators.js";
import { externalMarkdownUrl } from "../markdown";
import "../streaming-markdown";
import type { StreamingMarkdownState } from "../streaming-markdown";
import type { LensOutputBlock, LensState } from "../types";
import {
  imageDataUrl,
  lensOutputPresentation,
  type LensOutputMode,
  type LensOutputPresentation,
} from "../view-model";
import {
  AGENT_OUTPUT_INTENT_EVENT,
  dispatchComponentEvent,
  type AgentOutputIntent,
} from "./events";

export type ResponseNavigationRegion = "media" | "narrative";

@customElement("lens-agent-output")
export class LensAgentOutput extends LitElement {
  @property({ attribute: false })
  lens: LensState = { stage: "idle", prompt_execution_revision: 1, output_blocks: [] };
  @property({ attribute: false }) presentation: LensOutputPresentation | undefined;
  @property({ attribute: false }) htmlContent: HtmlOutputContent | undefined;
  @property({ attribute: false }) htmlContents: ReadonlyMap<string, HtmlOutputContent> | undefined;

  @property({ attribute: false }) history: ResponseHistoryPresentation | undefined;
  @property({ attribute: false }) loadResponseBlock: LoadResponseBlock | undefined;
  @property({ attribute: false }) requestMedia: ((ids: readonly string[]) => void) | undefined;
  @property({ attribute: false }) notificationContent: unknown;
  @property({ attribute: false }) retryMedia: ((id: string) => Promise<void>) | undefined;
  private navigationGeneration = 0;
  private historyOperation: string | undefined;
  private historyScrollTop = 0;
  private historyAnchor: { element: Element; offset: number } | undefined;
  private followHistoryEnd = false;
  private historyResize: ResizeObserver | undefined;
  private observedNarrative: Element | undefined;
  private historyScrollFrame: number | undefined;

  connectedCallback(): void {
    super.connectedCallback();
    this.requestUpdate();
  }

  disconnectedCallback(): void {
    ++this.navigationGeneration;
    super.disconnectedCallback();
    this.historyResize?.disconnect();
    this.historyResize = undefined;
    this.observedNarrative = undefined;
    if (this.historyScrollFrame !== undefined) cancelAnimationFrame(this.historyScrollFrame);
  }

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  private revealFirstMedia = false;

  protected willUpdate(changed: PropertyValues<this>): void {
    if (this.history?.responses.length) {
      if (this.historyOperation !== this.history.scopeId) {
        this.historyOperation = this.history.scopeId;
        this.historyScrollTop = 0;
        this.historyAnchor = undefined;
        this.followHistoryEnd = !this.history.media.length;
      } else {
        const output = this.querySelector<HTMLElement>(".lens-output");
        if (output) this.historyScrollTop = output.scrollTop;
      }
      this.revealFirstMedia = false;
      return;
    }
    if (!changed.has("lens") && !changed.has("presentation")) return;
    const previousLens = changed.has("lens") ? changed.get("lens") : this.lens;
    const previousPresentation = changed.has("presentation")
      ? changed.get("presentation")
      : this.presentation;
    const previousOutput =
      previousPresentation ?? (previousLens ? lensOutputPresentation(previousLens) : undefined);
    const output = this.presentation ?? lensOutputPresentation(this.lens);
    const hasMedia = composeOutputMedia(output).media.length > 0;
    const hadMedia = previousOutput ? composeOutputMedia(previousOutput).media.length > 0 : false;
    this.revealFirstMedia =
      hasMedia &&
      (!hadMedia ||
        (this.presentation
          ? previousOutput?.identity !== output.identity
          : previousLens?.operation_id !== this.lens.operation_id));
  }

  protected updated(): void {
    if (this.history?.responses.length) {
      const narrative = this.querySelector(".lens-output-narrative");
      if (
        narrative &&
        narrative !== this.observedNarrative &&
        typeof ResizeObserver !== "undefined"
      ) {
        this.historyResize?.disconnect();
        this.historyResize = new ResizeObserver(() => this.restoreHistoryScroll());
        this.historyResize.observe(narrative);
        this.observedNarrative = narrative;
      }
      this.restoreHistoryScroll();
      return;
    }
    if (!this.revealFirstMedia) return;
    this.revealFirstMedia = false;
    const output = this.querySelector<HTMLElement>(".lens-output");
    if (output) output.scrollTop = 0;
  }

  protected render() {
    if (this.history?.responses.length) return this.renderHistory();
    const output = this.presentation ?? lensOutputPresentation(this.lens);
    const { media, narrative } = composeOutputMedia(output);
    if (output.blocks.length) {
      return html`<div
        class="lens-content lens-output ${media.length ? "has-media" : ""} ${narrative.length ? "has-narrative" : ""}"
        data-auto-scroll-container
        role="document"
      >
        ${media.length ? html`<lens-output-media .media=${media} .htmlContent=${this.htmlContent} .htmlContents=${this.htmlContents} .notificationContent=${this.notificationContent}></lens-output-media>` : nothing}
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
    return this.presentation
      ? html`<p class="empty-state">No answer is available in this session.</p>`
      : this.renderEmpty();
  }

  private handleHistoryScroll = (): void => {
    const output = this.querySelector<HTMLElement>(".lens-output");
    if (!output) return;
    this.historyScrollTop = output.scrollTop;
    this.followHistoryEnd = output.scrollHeight - output.clientHeight - output.scrollTop <= 36;
    const top = output.getBoundingClientRect().top;
    const anchor = [...output.querySelectorAll("lens-response-block")].find(
      (block) => block.getBoundingClientRect().bottom > top,
    );
    this.historyAnchor = anchor
      ? { element: anchor, offset: anchor.getBoundingClientRect().top - top }
      : undefined;
  };

  private restoreHistoryScroll(): void {
    if (this.historyScrollFrame !== undefined) cancelAnimationFrame(this.historyScrollFrame);
    this.historyScrollFrame = requestAnimationFrame(() => {
      this.historyScrollFrame = undefined;
      const output = this.querySelector<HTMLElement>(".lens-output");
      if (!output) return;
      if (this.followHistoryEnd) output.scrollTop = output.scrollHeight;
      else if (this.historyAnchor?.element.isConnected) {
        const offset =
          this.historyAnchor.element.getBoundingClientRect().top -
          output.getBoundingClientRect().top;
        output.scrollTop += offset - this.historyAnchor.offset;
      } else output.scrollTop = this.historyScrollTop;
    });
  }

  currentResponseRegion(): ResponseNavigationRegion {
    const output = this.querySelector<HTMLElement>(".lens-output");
    const narrative = this.querySelector<HTMLElement>(".lens-output-narrative");
    if (!this.history?.media.length) return "narrative";
    if (!output || !narrative) return "media";
    return narrative.getBoundingClientRect().top <
      output.getBoundingClientRect().top + output.clientHeight / 2
      ? "narrative"
      : "media";
  }

  async revealResponse(
    scopeId: string,
    responseId: string,
    preferred: ResponseNavigationRegion,
  ): Promise<boolean> {
    const generation = ++this.navigationGeneration;
    const current = () =>
      this.isConnected &&
      generation === this.navigationGeneration &&
      this.history?.scopeId === scopeId;
    const response =
      this.history?.scopeId === scopeId
        ? this.history.responses.find((item) => item.id === responseId)
        : undefined;
    if (!response) return false;
    const mediaBlock = response.blocks.find(
      (block) => block.type === "image" || block.type === "html",
    );
    const narrativeBlock = response.blocks.find(
      (block) => block.type === "markdown" || block.type === "unsupported",
    );
    const media = this.querySelector<LensOutputMedia>("lens-output-media");
    const output = this.querySelector<HTMLElement>(".lens-output");
    if (!output) return false;
    if (media && !(await media.exitFullscreen())) return false;
    if (!current()) return false;
    this.followHistoryEnd = false;
    if (mediaBlock && (preferred === "media" || !narrativeBlock) && media) {
      this.historyAnchor = undefined;
      this.historyScrollTop = 0;
      output.scrollTop = 0;
      const mediaId = responseBlockIdentity(scopeId, responseId, mediaBlock.block_index);
      await this.retryMedia?.(mediaId);
      await this.updateComplete;
      await media.updateComplete;
      if (!current()) return false;
      const presented = await media.presentMedia(mediaId);
      if (!presented || !current()) return false;
      media
        .querySelector<HTMLButtonElement>(".output-media-details-toggle")
        ?.focus({ preventScroll: true });
      return true;
    }
    const section = [...this.querySelectorAll<HTMLElement>(".lens-response")].find(
      (item) => item.dataset.responseId === responseId,
    );
    const block = section?.querySelector<LensResponseBlock>("lens-response-block");
    if (!section || !block || !narrativeBlock) return false;
    const align = () => {
      this.historyAnchor = { element: section, offset: 0 };
      output.scrollTop += section.getBoundingClientRect().top - output.getBoundingClientRect().top;
      this.historyScrollTop = output.scrollTop;
    };
    align();
    const presented = await block.present();
    if (!presented || !current()) return false;
    align();
    section.tabIndex = -1;
    section.focus({ preventScroll: true });
    return true;
  }

  private renderHistory() {
    const history = this.history!;
    const narrativeResponses = history.responses.filter((response) =>
      response.blocks.some((block) => block.type === "markdown" || block.type === "unsupported"),
    );
    const hasNarrative = narrativeResponses.length > 0;
    return keyed(
      history.scopeId,
      html`<div
        class="lens-content lens-output ${history.media.length ? "has-media" : ""} ${hasNarrative ? "has-narrative" : ""}"
        data-auto-scroll-container
        role="document"
        @scroll=${this.handleHistoryScroll}
      >
        ${
          history.media.length
            ? html`<lens-output-media
                .notificationContent=${this.notificationContent}
                .media=${history.media}
                .htmlContents=${history.htmlContents}
                .mediaErrors=${history.mediaErrors}
                @lens-output-media-demand=${(
                  event: CustomEvent<{ mediaIds: readonly string[] }>,
                ) => {
                  event.stopPropagation();
                  this.requestMedia?.(event.detail.mediaIds);
                }}
              ></lens-output-media>`
            : nothing
        }
        ${history.media.length && hasNarrative ? html`<button type="button" class="output-media-explanation" @click=${this.showExplanation}>Explore the interpretation <i class="fa-solid fa-arrow-down" aria-hidden="true"></i></button>` : nothing}
        <div
          class="lens-output-narrative"
          @click=${this.openMarkdownLink}
          @markdown-render-error=${this.handleMarkdownRenderError}
        >
          ${history.media.length && hasNarrative ? html`<button type="button" class="output-media-return" @click=${this.showMedia}><i class="fa-solid fa-arrow-up" aria-hidden="true"></i> Back to media</button>` : nothing}
          ${repeat(
            narrativeResponses,
            (response) => response.id,
            (response) => html`<section
              class="lens-response"
              data-response-id=${response.id}
              data-response-sequence=${response.sequence}
              aria-label=${`Response ${response.sequence}`}
            >
              ${response.blocks.some((block) => block.type === "markdown" || block.type === "unsupported") ? html`<h2 class="lens-response-heading">Response ${response.sequence}</h2>` : nothing}
              ${repeat(
                response.blocks.filter(
                  (block) => block.type === "markdown" || block.type === "unsupported",
                ),
                (block) => block.block_index,
                (block) =>
                  html`<lens-response-block
                    .operationId=${history.scopeId}
                    .responseId=${response.id}
                    .descriptor=${block}
                    .loadBlock=${this.loadResponseBlock}
                  ></lens-response-block>`,
              )}
            </section>`,
          )}
          ${history.capacityReached ? html`<p class="lens-history-capacity" role="status">This session has reached its response history limit. Your previous responses remain available. Start a new session to continue.</p>` : nothing}
        </div>
      </div>`,
    );
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
