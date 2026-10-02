import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { LensOutputBlock, LensState } from "../contracts/lens";
import type {
  DeferredDocumentBlock,
  DocumentBlock,
  SessionView,
} from "../contracts/session-document";
import { presentMcpApps, type PresentedOutputMedia } from "../presentation/output-media";
import { imageDataUrl } from "../presentation/view-model";
import type {
  ResponseBodyPort,
  ResponseBlockDescriptor,
  ResponseManifest,
  ResponseHistoryPresentation,
  ProvisionalResponseBlock,
  LoadResponseBlock,
} from "../contracts/response-history";
export type {
  ResponseBlockDescriptor,
  ResponseManifest,
  ResponseHistoryPresentation,
  ProvisionalResponseBlock,
  LoadResponseBlock,
} from "../contracts/response-history";

export const RESPONSE_BODY_CACHE_BYTES = 16 * 1024 * 1024;
export const RESPONSE_BODY_CONCURRENCY = 2;
/** One response-scoped projection owns media identity for display and navigation. */
export function presentResponseMedia(
  scopeId: string,
  response: ResponseManifest,
): PresentedOutputMedia[] {
  const media: PresentedOutputMedia[] = [];
  for (const block of response.blocks) {
    const id = responseBlockIdentity(scopeId, response.id, block.block_index);
    if (block.type === "image") media.push({ kind: "image", id, mimeType: block.mime_type });
    else if (block.type === "html")
      media.push({
        kind: "html",
        id,
        resourceId: block.resource_id,
        mimeType: "text/html",
        uri: block.uri,
        byteLength: block.byte_length,
        presentationSource: block.source
          ? {
              kind: "history",
              generation: scopeId.slice("history:".length),
              entry_id: block.source.entry_id,
              revision: block.source.revision,
              block_index: block.source.block_index,
            }
          : {
              kind: "live",
              output_ref: { operation_id: scopeId, representation_id: response.id },
              block_index: block.block_index,
            },
      });
  }
  return [...media, ...presentMcpApps(response.mcpApps)];
}
interface ResponseManifestSet {
  responses: readonly ResponseManifest[];
  capacityReached: boolean;
}
export const responseBlockIdentity = (operation: string, response: string, index: number): string =>
  JSON.stringify([operation, response, index]);
interface CacheEntry {
  value: LensOutputBlock;
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
  private inflight = new Map<string, Promise<LensOutputBlock>>();
  private queue: PendingRequest[] = [];
  private active = 0;
  private mediaDemand = new Set<string>();
  private mediaBodies = new Map<string, LensOutputBlock>();
  private mediaErrors = new Map<string, string>();
  private initialProjection:
    | {
        status: "admitted" | "oversized";
        runId: string;
        blocks: readonly LensOutputBlock[];
        byteLengths: readonly number[];
      }
    | undefined;
  private provisionalBlocks = new Map<string, ProvisionalResponseBlock>();
  private provisionalBytes = 0;

  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly port: ResponseBodyPort,
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
    this.mediaErrors.clear();
    this.initialProjection = undefined;
    this.provisionalBlocks.clear();
    this.provisionalBytes = 0;
  }
  synchronize(lens: LensState | undefined): void {
    const history = lens?.response_history;
    const initial = lens?.operation_id === this.operation ? this.initialProjection : undefined;
    this.synchronizeSource(
      lens?.operation_id,
      history
        ? {
            responses: history.responses.map((response) => ({
              id: response.representation_id,
              mcpApps: response.mcp_apps,
              sequence: response.sequence,
              delivery: response.delivery,
              blocks: response.blocks,
            })),
            capacityReached: history.capacity_reached,
          }
        : undefined,
    );
    if (!lens || !history) return;
    if (!history.responses.length) {
      const runId = lens.agent?.run_id;
      if (lens.stage !== "transforming" || !runId) {
        this.initialProjection = undefined;
        return;
      }
      const previous = this.initialProjection?.runId === runId ? this.initialProjection : undefined;
      if (previous?.blocks === lens.output_blocks) return;
      const byteLengths = lens.output_blocks.map((block, index) => {
        const old = previous?.blocks[index];
        return old &&
          (old === block ||
            (old.type === "markdown" &&
              block.type === "markdown" &&
              old.text === block.text &&
              old.message_id === block.message_id))
          ? previous!.byteLengths[index]!
          : this.bodyBytes(block);
      });
      this.initialProjection = {
        status:
          byteLengths.reduce((total, bytes) => total + bytes, 0) <= RESPONSE_BODY_CACHE_BYTES
            ? "admitted"
            : "oversized",
        runId,
        blocks: lens.output_blocks,
        byteLengths,
      };
      return;
    }
    this.initialProjection = undefined;
    const first = history.responses[0];
    if (initial?.status !== "admitted" || first?.sequence !== 1 || first.run_id !== initial.runId)
      return;
    for (const descriptor of first.blocks) {
      const body = initial.blocks[descriptor.block_index];
      if (
        !body ||
        body.type !== descriptor.type ||
        (body.type !== "markdown" && body.type !== "image")
      )
        continue;
      if (
        body.type === "image" &&
        descriptor.type === "image" &&
        body.mime_type !== descriptor.mime_type
      )
        continue;
      const id = responseBlockIdentity(
        this.operation!,
        first.representation_id,
        descriptor.block_index,
      );
      const entry: ProvisionalResponseBlock = {
        body,
        bytes: initial.byteLengths[descriptor.block_index]!,
        release: () => this.releaseProvisional(id, entry),
      };
      this.provisionalBlocks.set(id, entry);
      this.provisionalBytes += entry.bytes;
    }
    this.publish();
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
              delivery: response.delivery,
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
    if (descriptor.type === "html")
      throw new Error("HTML display requires a native presentation lease.");
    return this.obtain(key, async () => {
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
    });
  };
  readonly requestMedia = (ids: readonly string[]): void => {
    const demand = new Set(ids);
    const changed =
      demand.size !== this.mediaDemand.size || [...demand].some((id) => !this.mediaDemand.has(id));
    if (!changed) return;
    this.mediaDemand = demand;
    for (const id of this.mediaBodies.keys()) if (!demand.has(id)) this.mediaBodies.delete(id);
    for (const id of this.mediaErrors.keys()) if (!demand.has(id)) this.mediaErrors.delete(id);
    for (const [id, entry] of this.provisionalBlocks)
      if (entry.body.type === "image" && !demand.has(id)) this.releaseProvisional(id, entry, false);
    const generation = this.generation;
    const operation = this.operation;
    if (!operation) return;
    for (const response of this.manifest?.responses ?? [])
      for (const descriptor of response.blocks) {
        const id = responseBlockIdentity(operation, response.id, descriptor.block_index);
        if (!demand.has(id) || descriptor.type !== "image") continue;
        if (this.mediaBodies.has(id)) continue;
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
      if (!descriptor || descriptor.type !== "image") continue;
      this.mediaErrors.delete(id);
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
      const value = await this.loadBlock(operation, response, descriptor.block_index);
      if (generation !== this.generation || !this.mediaDemand.has(id)) return;
      this.mediaErrors.delete(id);
      this.mediaBodies.set(id, value);
      const provisional = this.provisionalBlocks.get(id);
      if (provisional) this.releaseProvisional(id, provisional, false);
    } catch {
      if (generation !== this.generation || !this.mediaDemand.has(id)) return;
      const message = "This response media could not be loaded.";
      this.mediaErrors.set(id, message);
    }
    this.publish();
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
        throw new Error("Session content does not match its descriptor.");
      case "deferred":
        throw new Error("Session content was not materialized.");
    }
  }
  private obtain(key: string, load: () => Promise<LensOutputBlock>): Promise<LensOutputBlock> {
    const cached = this.cache.get(key);
    if (cached) {
      this.cache.delete(key);
      this.cache.set(key, cached);
      return Promise.resolve(cached.value);
    }
    const pending = this.inflight.get(key);
    if (pending) return pending;
    const generation = this.generation;
    const promise = new Promise<LensOutputBlock>((resolve, reject) => {
      this.queue.push({
        generation,
        reject,
        start: () => {
          this.active++;
          void load()
            .then((value) => {
              if (generation !== this.generation) throw new Error("Response is no longer active.");
              const bytes = new TextEncoder().encode(JSON.stringify(value)).byteLength;
              if (bytes > RESPONSE_BODY_CACHE_BYTES) {
                resolve(value);
                return;
              }
              while (
                this.cacheBytes + this.provisionalBytes + bytes > RESPONSE_BODY_CACHE_BYTES &&
                this.cache.size
              ) {
                const oldest = this.cache.keys().next().value!;
                this.cacheBytes -= this.cache.get(oldest)!.bytes;
                this.cache.delete(oldest);
              }
              if (this.cacheBytes + this.provisionalBytes + bytes > RESPONSE_BODY_CACHE_BYTES) {
                resolve(value);
                return;
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
  private bodyBytes(value: LensOutputBlock): number {
    // Native image bodies are ASCII base64; only their small metadata needs encoding.
    if (value.type === "image")
      return (
        new TextEncoder().encode(JSON.stringify({ ...value, data: "" })).byteLength +
        value.data.length
      );
    return new TextEncoder().encode(JSON.stringify(value)).byteLength;
  }
  private releaseProvisional(id: string, entry: ProvisionalResponseBlock, publish = true): void {
    if (this.provisionalBlocks.get(id) !== entry) return;
    this.provisionalBlocks.delete(id);
    this.provisionalBytes -= entry.bytes;
    if (publish) this.publish();
  }
  private publish(): void {
    if (!this.operation || !this.manifest) return;
    const operation = this.operation;
    const media = this.manifest.responses.flatMap((response) =>
      presentResponseMedia(operation, response).map((item) => {
        if (item.kind !== "image") return item;
        const body = this.mediaBodies.get(item.id) ?? this.provisionalBlocks.get(item.id)?.body;
        return {
          ...item,
          source: body?.type === "image" ? imageDataUrl(body) : undefined,
          provisional: !this.mediaBodies.has(item.id) && this.provisionalBlocks.has(item.id),
        };
      }),
    );
    this.presentation = {
      scopeId: operation,
      responses: this.manifest.responses,
      media,
      mediaErrors: new Map(this.mediaErrors),
      capacityReached: this.manifest.capacityReached,
      provisionalBlocks: new Map(this.provisionalBlocks),
    };
    this.host.requestUpdate();
  }
}
