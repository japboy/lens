// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { conversationTextChunks, LensConversationText } from "./lens-conversation-text";

afterEach(() => document.body.replaceChildren());
describe("plain conversation text", () => {
  it.each([
    "",
    "\r\n".repeat(100),
    "line\rnext\nend\r\n".repeat(100),
    "x".repeat(4095) + "😀" + "y".repeat(9000),
    "x".repeat(4095) + "\r\n" + "y".repeat(9000),
    "**bold** [link](javascript:alert(1))\n<script>alert(1)</script>\n```mermaid\ngraph LR; A-->B\n```",
  ])("preserves every character while bounding chunks (%#)", (source) => {
    const chunks = conversationTextChunks(source);
    expect(chunks.join("")).toBe(source);
    for (const chunk of chunks) {
      expect(chunk.length).toBeLessThanOrEqual(4096);
      expect([...chunk.matchAll(/\r\n|\r|\n/g)].length).toBeLessThanOrEqual(32);
      expect(chunk).not.toMatch(/[\ud800-\udbff]$/);
      expect(chunk).not.toMatch(/^[\udc00-\udfff]/);
    }
    for (let index = 1; index < chunks.length; index++)
      expect(chunks[index - 1]!.endsWith("\r") && chunks[index]!.startsWith("\n")).toBe(false);
  });
  it("bounds DOM size for a megabyte paragraph and preserves nodes on reconnect", async () => {
    const view = new LensConversationText();
    view.text = "**formatted** ".repeat(75000);
    document.body.append(view);
    await view.updateComplete;
    const chunks = [...view.shadowRoot!.querySelectorAll(".chunk")];
    expect(chunks).toHaveLength(Math.ceil(view.text.length / 4096));
    expect(view.shadowRoot!.querySelector(".text")!.textContent).toBe(view.text);
    expect(view.shadowRoot!.querySelector("strong")).toBeNull();
    view.remove();
    document.body.append(view);
    view.requestUpdate();
    await view.updateComplete;
    expect([...view.shadowRoot!.querySelectorAll(".chunk")]).toEqual(chunks);
    expect(view.shadowRoot!.querySelector(".chunk")).toBe(chunks[0]);
  });
  it("renders HTML and Markdown as inert literal text", async () => {
    const view = new LensConversationText();
    view.text = '<img src="https://example.com/track"><script>alert(1)</script> **bold**';
    document.body.append(view);
    await view.updateComplete;
    expect(view.shadowRoot!.querySelector(".text")!.textContent).toBe(view.text);
    expect(view.shadowRoot!.querySelector("img, script, strong, a, iframe")).toBeNull();
  });
});
