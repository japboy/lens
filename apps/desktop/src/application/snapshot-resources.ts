import type { AppSnapshot, LensOutputBlock } from "../types";
import type { LensOutputResource, LensSourceResource, WebviewPort } from "./webview-port";

export type ResourceState =
  | { stage: "idle" | "loading" | "ready" }
  | { stage: "failed"; message: string };

/** One in-flight request per resource, plus one replaceable latest reference. */
export class SnapshotResources {
  sourceState: ResourceState = { stage: "idle" };
  outputState: ResourceState = { stage: "idle" };
  private latest: AppSnapshot | undefined;
  private source: LensSourceResource | undefined;
  private output: LensOutputResource | undefined;
  private outputRunId: string | undefined;
  private images = new Map<string, string>();
  // At most the admitted image set and the latest in-flight resource's image set.
  private pendingImages = new Map<string, string>();
  private sourceBusy = false;
  private sourceDemand = false;
  private outputBusy = false;
  private generation = 0;
  private sourceAttempt: string | null | undefined;
  private outputAttempt: string | null | undefined;

  constructor(
    private readonly port: WebviewPort,
    private readonly changed: () => void,
  ) {}

  clear(): void {
    this.generation++;
    this.latest = undefined;
    this.source = undefined;
    this.output = undefined;
    this.outputRunId = undefined;
    this.images.clear();
    this.pendingImages.clear();
    this.sourceAttempt = this.outputAttempt = undefined;
    this.sourceState = this.outputState = { stage: "idle" };
  }

  synchronize(snapshot: AppSnapshot): void {
    if (this.latest?.lens.operation_id !== snapshot.lens.operation_id) this.clear();
    this.latest = snapshot;
    if (this.source?.source_ref !== snapshot.source_ref) this.source = undefined;
    if (!this.sourceDemand && this.sourceAttempt !== snapshot.source_ref)
      this.sourceState = { stage: "idle" };
    if (!snapshot.source_ref) {
      this.source = undefined;
      this.sourceAttempt = undefined;
      this.sourceState = { stage: "idle" };
    }
    if (!snapshot.output_ref) {
      this.output = undefined;
      this.outputRunId = undefined;
      this.outputAttempt = undefined;
      this.outputState = { stage: "idle" };
      this.images.clear();
      this.pendingImages.clear();
    }
    this.pumpSource();
    this.pumpOutput();
  }

  setSourceDemand(active: boolean): void {
    if (this.sourceDemand === active) return;
    this.sourceDemand = active;
    if (active) {
      if (this.source?.source_ref === this.latest?.source_ref && this.source)
        this.sourceState = { stage: "ready" };
      this.pumpSource();
    }
    this.changed();
  }

  retry(kind: "source" | "output"): void {
    if (kind === "source") {
      if (this.sourceBusy) return;
      this.sourceAttempt = undefined;
      this.pumpSource();
    } else {
      if (this.outputBusy) return;
      this.outputAttempt = undefined;
      this.pumpOutput();
    }
    this.changed();
  }

  project(snapshot: AppSnapshot): AppSnapshot {
    // Provisional blocks may remain visible only within the same run.
    const output =
      this.output?.output_ref === snapshot.output_ref ||
      (this.outputRunId !== undefined && this.outputRunId === snapshot.lens.agent?.run_id)
        ? this.output
        : undefined;
    return {
      ...snapshot,
      lens: {
        ...snapshot.lens,
        context:
          this.source && this.source.source_ref === snapshot.source_ref
            ? this.source.context
            : undefined,
        input:
          this.source && this.source.source_ref === snapshot.source_ref
            ? this.source.input
            : undefined,
        output_blocks: output?.output_blocks ?? [],
      },
    };
  }

  private pumpSource(): void {
    const ref = this.latest?.source_ref;
    if (!this.sourceDemand || this.sourceBusy || !ref || this.sourceAttempt === ref) return;
    this.sourceBusy = true;
    this.sourceAttempt = ref;
    this.sourceState = { stage: "loading" };
    const generation = this.generation;
    void this.port
      .getLensSource(ref)
      .then((resource) => {
        if (generation !== this.generation || this.latest?.source_ref !== ref) return;
        if (resource.source_ref !== ref) throw new Error("Source identity mismatch");
        this.source = resource;
        this.sourceState = { stage: "ready" };
      })
      .catch((error) => {
        if (generation === this.generation && this.latest?.source_ref === ref)
          this.sourceState = { stage: "failed", message: String(error) };
      })
      .finally(() => {
        this.sourceBusy = false;
        this.pumpSource();
        this.changed();
      });
  }

  private pumpOutput(): void {
    const ref = this.latest?.output_ref;
    if (this.outputBusy || !ref || this.outputAttempt === ref) return;
    this.outputBusy = true;
    this.outputAttempt = ref;
    this.outputState = { stage: "loading" };
    const generation = this.generation;
    const isCurrent = () => generation === this.generation && this.latest?.output_ref === ref;
    void this.port
      .getLensOutput(ref)
      .then(async (resource) => {
        if (!isCurrent()) return;
        if (resource.output_ref !== ref) throw new Error("Output identity mismatch");
        const imageRefs = new Set(
          resource.output_blocks.flatMap((block) =>
            block.type === "image" && block.image_ref ? [block.image_ref] : [],
          ),
        );
        const retained = this.pendingImages;
        for (const key of retained.keys()) {
          if (!imageRefs.has(key)) retained.delete(key);
        }
        const output_blocks: LensOutputBlock[] = [];
        for (const block of resource.output_blocks) {
          if (!isCurrent()) return;
          if (block.type !== "image" || !block.image_ref) {
            output_blocks.push(block);
            continue;
          }
          let data = retained.get(block.image_ref) ?? this.images.get(block.image_ref);
          if (data === undefined) {
            const image = await this.port.getLensImage(block.image_ref);
            if (image.image_ref !== block.image_ref) throw new Error("Image identity mismatch");
            data = image.data;
          }
          if (generation !== this.generation || !this.latest?.output_ref) return;
          // A newer text reference does not invalidate this immutable, operation-scoped image.
          retained.set(block.image_ref, data);
          output_blocks.push({ ...block, data });
        }
        if (!isCurrent()) return;
        this.images = retained;
        this.pendingImages = new Map();
        this.output = { ...resource, output_blocks };
        this.outputRunId = this.latest?.lens.agent?.run_id;
        this.outputState = { stage: "ready" };
      })
      .catch((error) => {
        if (isCurrent()) this.outputState = { stage: "failed", message: String(error) };
      })
      .finally(() => {
        this.outputBusy = false;
        this.pumpOutput();
        this.changed();
      });
  }
}
