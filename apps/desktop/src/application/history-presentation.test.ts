import { describe, expect, it } from "vitest";
import { historyPresentation } from "./history-presentation";
describe("history answer presentation", () => {
  it("renders selected result media and narrative without tool diagnostics", () => {
    const result = historyPresentation(
      {
        entries: [
          {
            id: "tool",
            kind: "tool",
            title: "publish",
            status: "completed",
            blocks: [
              { type: "markdown", text: "diagnostic" },
              { type: "html", text: "old html" },
            ],
            accepted_html: "new html",
          },
          {
            id: "a",
            kind: "message",
            role: "assistant",
            blocks: [{ type: "markdown", text: "answer" }],
          },
          {
            id: "failed",
            kind: "tool",
            title: "failed",
            status: "failed",
            blocks: [{ type: "html", text: "failed html" }],
          },
        ],
      },
      "saved",
    );
    expect(result.presentation.published).toBeUndefined();
    expect(result.presentation.blocks.map((block) => block.type)).toEqual([
      "html",
      "html",
      "markdown",
    ]);
    expect([...result.htmlContents.values()]).toEqual([
      expect.objectContaining({ status: "ready", content: "old html" }),
      expect.objectContaining({ status: "ready", content: "new html" }),
    ]);
    expect(result.htmlContent).toBeUndefined();
    expect(JSON.stringify(result)).not.toContain("diagnostic");
    expect(JSON.stringify(result)).not.toContain("failed html");
  });
  it("renders an empty backend selection without inventing an answer", () => {
    expect(historyPresentation({ entries: [] }, "s").presentation.mode).toBe("empty");
    expect(historyPresentation(undefined, "s").presentation.blocks).toEqual([]);
  });
});
