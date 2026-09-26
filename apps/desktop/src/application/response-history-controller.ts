import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { LensOutputBlock, LensResponseBlockDescriptor, LensState } from "../types";
import type { DeferredDocumentBlock, DocumentBlock, SessionView } from "./session-document";
import type { PresentedOutputMedia } from "../output-media";
import { imageDataUrl } from "../view-model";
import { MAX_HTML_OUTPUT_BYTES, type HtmlOutputContent } from "./html-output-controller";
import type { WebviewPort } from "./webview-port";

export const RESPONSE_BODY_CACHE_BYTES = 16 * 1024 * 1024;
export const RESPONSE_BODY_CONCURRENCY = 2;
export type ResponseBlockDescriptor = LensResponseBlockDescriptor & {
  source?: DeferredDocumentBlock;
};
export interface ResponseManifest {
  id: string;
  sequence: number;
  blocks: readonly ResponseBlockDescriptor[];
}
interface ResponseManifestSet {
  responses: readonly ResponseManifest[];
  capacityReached: boolean;
}
export interface ResponseHistoryPresentation {
  scopeId: string;
  responses: readonly ResponseManifest[];
  media: readonly PresentedOutputMedia[];
  htmlContents: ReadonlyMap<string, HtmlOutputContent>;
  mediaErrors: ReadonlyMap<string, string>;
  capacityReached: boolean;
}
export type LoadResponseBlock = (
  operationId: string,
  representationId: string,
  blockIndex: number,
) => Promise<LensOutputBlock>;
export const responseBlockIdentity = (operation: string, response: string, index: number): string =>
  JSON.stringify([operation, response, index]);
interface CacheEntry {
  value: LensOutputBlock | string;
  bytes: number;
}
interface PendingRequest {
  generation: number;
  start: () => void;
  reject: (reason: Error) => void;
}

/** Manifest authority is independent from bounded, demand-loaded presentation bodies. */
export class ResponseHistoryController implements ReactiveController {
  presentation: ResponseHistoryPresentation | undefined;
  private operation: string | undefined;
  private manifest: ResponseManifestSet | undefined;
  private generation = 0;
  private cache = new Map<string, CacheEntry>();
  private cacheBytes = 0;
  private inflight = new Map<string, Promise<LensOutputBlock | string>>();
  private queue: PendingRequest[] = [];
  private active = 0;
  private mediaDemand = new Set<string>();
  private mediaBodies = new Map<string, LensOutputBlock>();
  private htmlContents = new Map<string, HtmlOutputContent>();
  private mediaErrors = new Map<string, string>();

  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly port: Pick<WebviewPort, "getResponseBlock" | "getHtmlOutput">,
    private readonly loadHistoryBlock?: (
      reference: DeferredDocumentBlock,
    ) => Promise<DocumentBlock>,
  ) {
    host.addController(this);
  }
  hostDisconnected(): void {
    this.reset();
    this.operation = undefined;
    this.manifest = undefined;
    this.presentation = undefined;
  }
  private reset(): void {
    ++this.generation;
    this.cache.clear();
    this.cacheBytes = 0;
    this.inflight.clear();
    for (const request of this.queue) request.reject(new Error("Response is no longer active."));
    this.queue = [];
    this.mediaDemand.clear();
    this.mediaBodies.clear();
    this.htmlContents.clear();
    this.mediaErrors.clear();
  }
  synchronize(lens: LensState | undefined): void {
    const history = lens?.response_history;
    this.synchronizeSource(
      lens?.operation_id,
      history
        ? {
            responses: history.responses.map((response) => ({
              id: response.representation_id,
              sequence: response.sequence,
              blocks: response.blocks,
            })),
            capacityReached: history.capacity_reached,
          }
        : undefined,
    );
  }
  synchronizeHistory(view: SessionView | undefined): void {
    const interpretation = view?.phase === "ready" ? view.interpretation : undefined;
    this.synchronizeSource(
      view?.generation ? `history:${view.generation}` : undefined,
      interpretation
        ? {
            responses: interpretation.responses.map((response) => ({
              id: response.response_id,
              sequence: response.sequence,
              blocks: response.blocks,
            })),
            capacityReached: false,
          }
        : undefined,
    );
  }
  private synchronizeSource(
    operation: string | undefined,
    manifest: ResponseManifestSet | undefined,
  ): void {
    if (operation !== this.operation) {
      this.reset();
      this.operation = operation;
      this.manifest = undefined;
    }
    if (!manifest || !operation) {
      if (this.manifest) this.reset();
      this.manifest = undefined;
      this.presentation = undefined;
      return;
    }
    // Complete manifests reconstruct both live publication and immutable replay.
    if (this.manifest && manifest.responses.length < this.manifest.responses.length) return;
    if (
      this.manifest?.responses.length === manifest.responses.length &&
      this.manifest.capacityReached === manifest.capacityReached
    )
      return;
    this.manifest = manifest;
    this.publish();
  }
  private descriptor(operation: string, response: string, index: number): ResponseBlockDescriptor {
    if (operation !== this.operation) throw new Error("Response is no longer active.");
    const descriptor = this.manifest?.responses
      .find((r) => r.id === response)
      ?.blocks.find((b) => b.block_index === index);
    if (!descriptor) throw new Error("Response block is unavailable.");
    return descriptor;
  }
  readonly loadBlock: LoadResponseBlock = async (operation, response, index) => {
    const descriptor = this.descriptor(operation, response, index);
    const key = responseBlockIdentity(operation, response, index);
    return (await this.obtain(key, async () => {
      const block = descriptor.source
        ? await this.loadReplayBlock(descriptor)
        : await this.port.getResponseBlock(operation, response, index);
      if (block.type !== descriptor.type)
        throw new Error("Response block does not match its descriptor.");
      if (
        block.type === "markdown" &&
        descriptor.type === "markdown" &&
        new TextEncoder().encode(block.text).byteLength !== descriptor.byte_length
      )
        throw new Error("Response text does not match its descriptor.");
      if (
        block.type === "image" &&
        descriptor.type === "image" &&
        (block.mime_type !== descriptor.mime_type || block.data.length !== descriptor.byte_length)
      )
        throw new Error("Response image does not match its descriptor.");
      return block;
    })) as LensOutputBlock;
  };
  readonly requestMedia = (ids: readonly string[]): void => {
    const demand = new Set(ids);
    const changed =
      demand.size !== this.mediaDemand.size || [...demand].some((id) => !this.mediaDemand.has(id));
    if (!changed) return;
    this.mediaDemand = demand;
    for (const id of this.mediaBodies.keys()) if (!demand.has(id)) this.mediaBodies.delete(id);
    for (const id of this.htmlContents.keys()) if (!demand.has(id)) this.htmlContents.delete(id);
    for (const id of this.mediaErrors.keys()) if (!demand.has(id)) this.mediaErrors.delete(id);
    const generation = this.generation;
    const operation = this.operation;
    if (!operation) return;
    for (const response of this.manifest?.responses ?? [])
      for (const descriptor of response.blocks) {
        const id = responseBlockIdentity(operation, response.id, descriptor.block_index);
        if (!demand.has(id) || (descriptor.type !== "image" && descriptor.type !== "html"))
          continue;
        if (this.mediaBodies.has(id) || this.htmlContents.has(id)) continue;
        if (descriptor.type === "html")
          this.htmlContents.set(id, { resourceId: descriptor.resource_id, status: "loading" });
        void this.materializeMedia(operation, response.id, descriptor, id, generation);
      }
    this.publish();
  };
  /** Only an explicit user retry reopens failed demand; snapshots never retry it. */
  readonly retryMedia = async (id: string): Promise<void> => {
    if (!this.mediaErrors.has(id) || !this.operation) return;
    const operation = this.operation;
    for (const response of this.manifest?.responses ?? []) {
      const descriptor = response.blocks.find(
        (block) => responseBlockIdentity(operation, response.id, block.block_index) === id,
      );
      if (!descriptor || (descriptor.type !== "image" && descriptor.type !== "html")) continue;
      this.mediaErrors.delete(id);
      if (descriptor.type === "html")
        this.htmlContents.set(id, { resourceId: descriptor.resource_id, status: "loading" });
      this.publish();
      await this.materializeMedia(operation, response.id, descriptor, id, this.generation);
      return;
    }
  };
  private async materializeMedia(
    operation: string,
    response: string,
    descriptor: ResponseBlockDescriptor,
    id: string,
    generation: number,
  ): Promise<void> {
    try {
      const value = await this.loadMedia(operation, response, descriptor, id);
      if (generation !== this.generation || !this.mediaDemand.has(id)) return;
      this.mediaErrors.delete(id);
      if (typeof value === "string" && descriptor.type === "html")
        this.htmlContents.set(id, {
          resourceId: descriptor.resource_id,
          status: "ready",
          content: value,
        });
      else if (typeof value !== "string") this.mediaBodies.set(id, value);
    } catch {
      if (generation !== this.generation || !this.mediaDemand.has(id)) return;
      const message = "This response media could not be loaded.";
      this.mediaErrors.set(id, message);
      if (descriptor.type === "html")
        this.htmlContents.set(id, {
          resourceId: descriptor.resource_id,
          status: "failed",
          message,
        });
    }
    this.publish();
  }
  private async loadMedia(
    operation: string,
    response: string,
    descriptor: ResponseBlockDescriptor,
    id: string,
  ): Promise<string | LensOutputBlock> {
    if (descriptor.type !== "html")
      return this.loadBlock(operation, response, descriptor.block_index);
    if (
      !Number.isSafeInteger(descriptor.byte_length) ||
      descriptor.byte_length < 0 ||
      descriptor.byte_length > MAX_HTML_OUTPUT_BYTES
    )
      throw new Error("HTML content exceeds the supported size.");
    return this.obtain(`${id}:html`, async () => {
      let content: string;
      if (descriptor.source) {
        if (!this.loadHistoryBlock) throw new Error("Session block loader unavailable.");
        const block = await this.loadHistoryBlock(descriptor.source);
        if (block.type !== "html") throw new Error("Session HTML does not match its descriptor.");
        content = block.text;
      } else content = await this.port.getHtmlOutput(operation, response, descriptor.resource_id);
      if (new TextEncoder().encode(content).byteLength !== descriptor.byte_length)
        throw new Error("HTML content does not match its descriptor.");
      return content;
    });
  }
  private async loadReplayBlock(descriptor: ResponseBlockDescriptor): Promise<LensOutputBlock> {
    if (!descriptor.source || !this.loadHistoryBlock)
      throw new Error("Session block loader unavailable.");
    const block = await this.loadHistoryBlock(descriptor.source);
    switch (block.type) {
      case "markdown":
      case "image":
      case "unsupported":
        return block;
      case "html":
        if (descriptor.type !== "html")
          throw new Error("Session HTML does not match its descriptor.");
        return {
          type: "html",
          resource_id: descriptor.resource_id,
          uri: descriptor.uri,
          mime_type: descriptor.mime_type,
          byte_length: descriptor.byte_length,
        };
      case "deferred":
        throw new Error("Session content was not materialized.");
    }
  }
  private obtain(
    key: string,
    load: () => Promise<LensOutputBlock | string>,
  ): Promise<LensOutputBlock | string> {
    const cached = this.cache.get(key);
    if (cached) {
      this.cache.delete(key);
      this.cache.set(key, cached);
      return Promise.resolve(cached.value);
    }
    const pending = this.inflight.get(key);
    if (pending) return pending;
    const generation = this.generation;
    const promise = new Promise<LensOutputBlock | string>((resolve, reject) => {
      this.queue.push({
        generation,
        reject,
        start: () => {
          this.active++;
          void load()
            .then((value) => {
              if (generation !== this.generation) throw new Error("Response is no longer active.");
              const bytes = new TextEncoder().encode(
                typeof value === "string" ? value : JSON.stringify(value),
              ).byteLength;
              if (bytes > RESPONSE_BODY_CACHE_BYTES) {
                resolve(value);
                return;
              }
              while (this.cacheBytes + bytes > RESPONSE_BODY_CACHE_BYTES && this.cache.size) {
                const oldest = this.cache.keys().next().value!;
                this.cacheBytes -= this.cache.get(oldest)!.bytes;
                this.cache.delete(oldest);
              }
              this.cache.set(key, { value, bytes });
              this.cacheBytes += bytes;
              resolve(value);
            })
            .catch(reject)
            .finally(() => {
              this.active--;
              if (this.inflight.get(key) === promise) this.inflight.delete(key);
              this.pump();
            });
        },
      });
    });
    this.inflight.set(key, promise);
    this.pump();
    return promise;
  }
  private pump(): void {
    while (this.active < RESPONSE_BODY_CONCURRENCY && this.queue.length) {
      const request = this.queue.shift()!;
      if (request.generation === this.generation) request.start();
      else request.reject(new Error("Response is no longer active."));
    }
  }
  private publish(): void {
    if (!this.operation || !this.manifest) return;
    const operation = this.operation;
    const media: PresentedOutputMedia[] = [];
    for (const response of this.manifest.responses)
      for (const block of response.blocks) {
        const id = responseBlockIdentity(operation, response.id, block.block_index);
        if (block.type === "image") {
          const body = this.mediaBodies.get(id);
          media.push({
            kind: "image",
            id,
            mimeType: block.mime_type,
            source: body?.type === "image" ? imageDataUrl(body) : undefined,
          });
        } else if (block.type === "html")
          media.push({
            kind: "html",
            id,
            resourceId: block.resource_id,
            mimeType: "text/html",
            uri: block.uri,
            byteLength: block.byte_length,
          });
      }
    this.presentation = {
      scopeId: operation,
      responses: this.manifest.responses,
      media,
      htmlContents: new Map(this.htmlContents),
      mediaErrors: new Map(this.mediaErrors),
      capacityReached: this.manifest.capacityReached,
    };
    this.host.requestUpdate();
  }
}
