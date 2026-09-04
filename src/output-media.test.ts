import { describe, expect, it } from "vitest";
import { composeOutputMedia } from "./output-media";
import { lensOutputPresentation } from "./view-model";

describe("output media composition", () => {
  it("promotes standalone typed images while retaining narrative order and inline Markdown", () => {
    const output = {
      identity: "settled-1",
      mode: "settled" as const,
      blocks: [
        { type: "markdown" as const, text: "Before ![inline](https://example.com/chart.png)" },
        { type: "image" as const, mime_type: "image/png", data: "aGVsbG8=" },
        { type: "markdown" as const, text: "After" },
        { type: "image" as const, mime_type: "image/webp", data: "d29ybGQ=" },
        { type: "image" as const, mime_type: "image/svg+xml", data: "not admitted" },
      ],
    };
    const composition = composeOutputMedia(output);
    expect(composition.media.map((item) => item.id)).toEqual([
      "settled-1:image:1",
      "settled-1:image:3",
    ]);
    expect(composition.narrative.map(({ index }) => index)).toEqual([0, 2, 4]);
    expect(composition.narrative[0]?.block).toBe(output.blocks[0]);
    expect(composition.media[0]?.source).toBe("data:image/png;base64,aGVsbG8=");
    expect(composeOutputMedia({ ...output, identity: "settled-2" }).media[0]?.id).not.toBe(
      composition.media[0]?.id,
    );
  });

  it("derives the gallery only from the published representation when a candidate exists", () => {
    const output = lensOutputPresentation({
      operation_id: "operation",
      stage: "transforming",
      output_blocks: [{ type: "image", mime_type: "image/png", data: "private candidate" }],
      representation: {
        representation_id: "published",
        context_id: "context",
        context_revision: 1,
        projection: { revision: 1, digest: "sha256:published" },
        run_id: "run",
        output_blocks: [{ type: "markdown", text: "Published text only" }],
      },
    });
    expect(composeOutputMedia(output).media).toEqual([]);
    expect(composeOutputMedia(output).narrative[0]?.block).toEqual({
      type: "markdown",
      text: "Published text only",
    });
  });
});
