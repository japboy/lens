import { describe, expect, it } from "vitest";
import type { ReactiveControllerHost } from "lit";
import { ResponseHistoryController } from "./response-history-controller";
import { mediaCases, mediaFixture, artifact } from "../../tests/fixtures/media-parity";

describe("live/replay media contract", () => {
  it.each(mediaCases)("preserves %s media, ordering, and narrative", async (choice) => {
    const fixture = mediaFixture(choice);
    const host = { addController() {}, requestUpdate() {} } as unknown as ReactiveControllerHost;
    const live = new ResponseHistoryController(host, fixture.port);
    const replay = new ResponseHistoryController(host, fixture.port, fixture.loadSessionBlock);
    live.synchronize(fixture.lens);
    replay.synchronizeHistory({
      revision: 1,
      phase: "ready",
      generation: "replay",
      interpretation: fixture.interpretation,
    });
    const expected = fixture.lens.response_history.responses[0]!.blocks;
    for (const controller of [live, replay]) {
      const presentation = controller.presentation!;
      expect(presentation.responses[0]!.blocks.map((block) => block.type)).toEqual(
        expected.map((block) => block.type),
      );
      expect(presentation.media.map((media) => media.kind)).toEqual(
        choice === "none"
          ? []
          : choice === "single"
            ? ["image"]
            : choice === "mixed"
              ? ["image", "image", "image", "html"]
              : ["image", "image", "image"],
      );
      controller.requestMedia(presentation.media.map((media) => media.id));
      for (const block of expected.filter((block) => block.type === "markdown")) {
        expect(
          await controller.loadBlock(
            presentation.scopeId,
            presentation.responses[0]!.id,
            block.block_index,
          ),
        ).toEqual(await fixture.port.getResponseBlock("", "", block.block_index));
      }
      await expect
        .poll(
          () =>
            controller.presentation!.media.filter((media) => media.kind === "image" && media.source)
              .length,
        )
        .toBe(choice === "none" ? 0 : choice === "single" ? 1 : 3);
      const expectedHtml =
        choice === "mixed"
          ? [{ resourceId: "fixture-html", status: "ready", content: artifact }]
          : [];
      await expect
        .poll(() => [...controller.presentation!.htmlContents.values()])
        .toEqual(expectedHtml);
    }
  });
});
