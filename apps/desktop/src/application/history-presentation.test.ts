import { describe, expect, it } from "vitest";
import { historyPresentation } from "./history-presentation";
describe("history answer presentation", () => {
  it("keeps final answer media and narrative without tool diagnostics or prior turns", () => {
    const result = historyPresentation(
      {
        entries: [
          {
            id: "old",
            kind: "message",
            role: "assistant",
            blocks: [{ type: "markdown", text: "old" }],
          },
          {
            id: "user",
            kind: "message",
            role: "user",
            blocks: [{ type: "markdown", text: "question" }],
          },
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
    expect(result.presentation.blocks.map((block) => block.type)).toEqual(["html", "markdown"]);
    expect(result.htmlContent).toMatchObject({ status: "ready", content: "new html" });
    expect(JSON.stringify(result)).not.toContain("diagnostic");
    expect(JSON.stringify(result)).not.toContain("failed html");
  });
  it("does not resurrect an earlier answer after a final unanswered prompt", () => {
    expect(
      historyPresentation(
        {
          entries: [
            {
              id: "a",
              kind: "message",
              role: "assistant",
              blocks: [{ type: "markdown", text: "old" }],
            },
            { id: "u", kind: "message", role: "user", blocks: [] },
          ],
        },
        "s",
      ).presentation.blocks,
    ).toEqual([]);
  });
});
