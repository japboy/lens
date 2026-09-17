import { describe, expect, it } from "vitest";
import { historyPresentation } from "./history-presentation";
import { composeOutputMedia } from "../output-media";
import { lensOutputPresentation } from "../view-model";
import { mediaCases, mediaFixture, artifact } from "../../tests/fixtures/media-parity";

describe("normal/history media contract", () => {
  it.each(mediaCases)("preserves %s media, ordering, and narrative", (choice) => {
    const fixture = mediaFixture(choice);
    const normal = lensOutputPresentation(fixture.lens);
    const history = historyPresentation(fixture.document, "history");
    expect(history.presentation.blocks.map((b) => b.type)).toEqual(
      normal.blocks.map((b) => b.type),
    );
    expect(history.presentation.blocks.filter((b) => b.type !== "html")).toEqual(
      normal.blocks.filter((b) => b.type !== "html"),
    );
    expect(normal.blocks.filter((b) => b.type === "image")).toHaveLength(
      choice === "none" ? 0 : choice === "single" ? 1 : 3,
    );
    expect(normal.blocks.filter((b) => b.type === "html")).toHaveLength(choice === "mixed" ? 1 : 0);
    expect(history.htmlContent?.status === "ready" ? history.htmlContent.content : undefined).toBe(
      choice === "mixed" ? artifact : undefined,
    );
    expect(history.presentation.mode).toBe(normal.mode);
    const normalMedia = composeOutputMedia(normal);
    const historyMedia = composeOutputMedia(history.presentation);
    expect(historyMedia.media.map((item) => item.kind)).toEqual(
      normalMedia.media.map((item) => item.kind),
    );
    expect(normalMedia.media).toHaveLength(
      choice === "none" ? 0 : choice === "single" ? 1 : choice === "mixed" ? 4 : 3,
    );
    expect(historyMedia.narrative).toEqual(normalMedia.narrative);
  });
});
