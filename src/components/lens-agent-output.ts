import { LitElement, html } from "lit";
import { customElement, property } from "lit/decorators.js";
import { externalMarkdownUrl } from "../markdown";
import "../streaming-markdown";
import type { StreamingMarkdownState } from "../streaming-markdown";
import type { LensOutputBlock, LensState } from "../types";
import { imageDataUrl, lensOutputBlocks, showsLensProgress, STAGE_LABEL } from "../view-model";
import {
  AGENT_OUTPUT_INTENT_EVENT,
  dispatchComponentEvent,
  type AgentOutputIntent,
} from "./events";

@customElement("lens-agent-output")
export class LensAgentOutput extends LitElement {
  @property({ attribute: false })
  lens: LensState = { stage: "idle", output_blocks: [] };

  protected createRenderRoot(): HTMLElement {
    return this;
  }

  protected render() {
    const blocks = lensOutputBlocks(this.lens);
    if (blocks.length) {
      return html`<div
        class="lens-content lens-output"
        data-auto-scroll-container
        role="document"
        aria-live="polite"
      >
        ${blocks.map((block, index) => this.renderBlock(block, index === blocks.length - 1))}
      </div>`;
    }
    return showsLensProgress(this.lens.stage) ? this.renderLoading() : this.renderEmpty();
  }

  private renderBlock(block: LensOutputBlock, isLastBlock: boolean) {
    switch (block.type) {
      case "markdown":
        return html`<lens-markdown
          class="markdown-body"
          .state=${
            {
              operationId: this.lens.operation_id,
              markdown: block.text,
              phase: this.lens.stage === "transforming" && isLastBlock ? "streaming" : "settled",
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
    }
  }

  private renderUnsupported(contentType: string) {
    return html`<p class="lens-output-unsupported" role="note">
      This agent output type is not supported yet: <code>${contentType}</code>
    </p>`;
  }

  private renderLoading() {
    return html`
      <div class="loading-state" role="status" aria-live="polite">
        <i class="fa-solid fa-spinner fa-spin" aria-hidden="true"></i>
        <strong>${STAGE_LABEL[this.lens.stage]}</strong>
        <p>The source remains available in its own tab while Lens prepares the result.</p>
      </div>
    `;
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
          return "The Agent did not produce a translation.";
        case "completed":
          return "The Agent completed without returning displayable content.";
        case "selecting":
        case "extracting":
        case "ready":
        case "connecting":
        case "transforming":
          return "Preparing the Agent translation.";
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
