// @vitest-environment jsdom
import { afterEach, beforeAll, expect, it } from "vitest";
import type { LensMediaGeometry } from "../../contracts/lens";
import type { LensMediaGallery } from "./lens-media-gallery";

beforeAll(async () => {
  await import("./lens-media-gallery");
});
afterEach(() => document.body.replaceChildren());

async function render(geometry: LensMediaGeometry): Promise<LensMediaGallery> {
  const element = document.createElement("lens-media-gallery") as LensMediaGallery;
  const operation = "00000000-0000-0000-0000-000000000001";
  element.lens = {
    operation_id: operation,
    stage: "ready",
    prompt_execution_revision: 1,
    response_history: { responses: [], retained_bytes: 0, capacity_reached: false },
    output_blocks: [],
    input: {
      schema_version: 5,
      context_id: operation,
      context_revision: 1,
      sources: [],
      media: [
        {
          id: "image",
          target_id: "target",
          uri: `lens://context/${operation}/1/media/image`,
          scope: "window_fallback",
          geometry,
          coverage: "full_region",
          mime_type: "image/png",
          pixel_width: 50,
          pixel_height: 40,
          encoded_bytes: 100,
        },
      ],
      media_omissions: [],
      quality: "full",
    },
  };
  document.body.append(element);
  await element.updateComplete;
  return element;
}

it("distinguishes source crop and encoded pixels without implying available desktop geometry", async () => {
  const element = await render({
    provenance: "acquired",
    frame: {
      id: "00000000-0000-0000-0000-000000000002",
      extent: { width: 200, height: 160 },
      content_bounds: { x: 0, y: 0, width: 200, height: 160 },
      desktop_mapping: { status: "unavailable", reason: "unverified_configuration" },
    },
    source_bounds: { x: 0, y: 0, width: 200, height: 160 },
    captured_bounds: { x: 0, y: 0, width: 200, height: 160 },
    crop: { x: 0, y: 0, width: 200, height: 160 },
  });
  const metadata = element
    .querySelector(".input-media-metadata")!
    .textContent!.replace(/\s+/g, " ");
  expect(metadata).toContain("Encoded pixels 50 × 40");
  expect(metadata).toContain("Source frame pixels 200 × 160");
  expect(metadata).toContain("Source crop pixels 0, 0 · 200 × 160");
  expect(metadata).toContain("Coordinates frame_pixels");
  expect(metadata).toContain("Unavailable: unverified_configuration");
  expect(metadata).not.toContain("desktop_logical");
});

it("labels historical coordinates unproven and does not invent a source frame or crop", async () => {
  const element = await render({
    provenance: "historical_unproven",
    source_bounds: { x: -10, y: 20, width: 100, height: 80 },
    captured_bounds: { x: 0, y: 20, width: 90, height: 80 },
    coordinate_space: "screen_points",
  });
  const metadata = element
    .querySelector(".input-media-metadata")!
    .textContent!.replace(/\s+/g, " ");
  expect(metadata).toContain("screen_points (historical; unproven)");
  expect(metadata).not.toContain("Source frame pixels");
  expect(metadata).not.toContain("Source crop pixels");
  expect(metadata).not.toContain("Desktop mapping");
});
