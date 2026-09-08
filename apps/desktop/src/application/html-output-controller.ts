import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { LensState } from "../types";
import type { WebviewPort } from "./webview-port";

export type HtmlOutputContent =
  | { resourceId: string; status: "loading" }
  | { resourceId: string; status: "ready"; content: string }
  | { resourceId: string; status: "failed"; message: string };

export const MAX_HTML_OUTPUT_BYTES = 512 * 1024;

/** Loads only committed resources; async results cannot cross representation boundaries. */
export class HtmlOutputController implements ReactiveController {
  content: HtmlOutputContent | undefined;
  private identity: string | undefined;
  private generation = 0;

  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly port: Pick<WebviewPort, "getHtmlOutput">,
  ) {
    host.addController(this);
  }

  hostDisconnected(): void {
    this.generation += 1;
    this.identity = undefined;
    this.content = undefined;
  }

  synchronize(lens: LensState | undefined): void {
    const representation = lens?.representation;
    const resources = representation?.output_blocks.filter((block) => block.type === "html") ?? [];
    const resource = resources[0];
    const operationId = lens?.operation_id;
    const identity =
      operationId && representation && resource
        ? JSON.stringify([operationId, representation.representation_id, resource.resource_id])
        : undefined;
    if (identity === this.identity) return;
    this.identity = identity;
    const generation = ++this.generation;
    if (!identity || !operationId || !representation || !resource) {
      this.setContent(undefined);
      return;
    }
    if (resources.length !== 1) {
      this.setContent({
        resourceId: resource.resource_id,
        status: "failed",
        message: "Multiple HTML resources are not supported.",
      });
      return;
    }
    if (
      !Number.isSafeInteger(resource.byte_length) ||
      resource.byte_length < 0 ||
      resource.byte_length > MAX_HTML_OUTPUT_BYTES
    ) {
      this.setContent({
        resourceId: resource.resource_id,
        status: "failed",
        message: "HTML content exceeds the supported size.",
      });
      return;
    }
    this.setContent({ resourceId: resource.resource_id, status: "loading" });
    void this.load(
      operationId,
      representation.representation_id,
      resource.resource_id,
      resource.byte_length,
      generation,
    );
  }

  private async load(
    operationId: string,
    representationId: string,
    resourceId: string,
    byteLength: number,
    generation: number,
  ): Promise<void> {
    try {
      const content = await this.port.getHtmlOutput(operationId, representationId, resourceId);
      if (generation !== this.generation) return;
      const actualBytes = new TextEncoder().encode(content).byteLength;
      if (actualBytes > MAX_HTML_OUTPUT_BYTES || actualBytes !== byteLength) {
        throw new Error("HTML content does not match its resource descriptor.");
      }
      this.setContent({ resourceId, status: "ready", content });
    } catch {
      if (generation !== this.generation) return;
      this.setContent({
        resourceId,
        status: "failed",
        message: "HTML content could not be loaded.",
      });
    }
  }

  private setContent(content: HtmlOutputContent | undefined): void {
    this.content = content;
    this.host.requestUpdate();
  }
}
