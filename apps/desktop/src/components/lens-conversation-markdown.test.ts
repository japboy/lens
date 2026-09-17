// @vitest-environment jsdom
import { afterEach, expect, it } from "vitest";
import { LensConversationMarkdown } from "./lens-conversation-markdown";
afterEach(() => document.body.replaceChildren());
it("renders ordinary Markdown and leaves Mermaid as fenced code", () => {
  const view = new LensConversationMarkdown();
  view.markdown = "# Answer\n\n**bold**\n\n```mermaid\ngraph LR; A-->B\n```";
  document.body.append(view);
  expect(view.querySelector("h1")?.textContent).toBe("Answer");
  expect(view.querySelector("strong")?.textContent).toBe("bold");
  expect(view.querySelector("code.language-mermaid")?.textContent).toContain("graph LR");
  expect(view.querySelector("svg")).toBeNull();
});
it("treats raw HTML as text and rejects executable links", () => {
  const view = new LensConversationMarkdown();
  view.markdown =
    "<script>alert(1)</script>\n\n[bad](javascript:alert) [good](https://example.test)";
  document.body.append(view);
  expect(view.querySelector("script")).toBeNull();
  expect(view.textContent).toContain("<script>");
  expect(
    [...view.querySelectorAll("a")].some((link) =>
      link.getAttribute("href")?.startsWith("javascript:"),
    ),
  ).toBe(false);
  expect(view.querySelector('a[href="https://example.test"]')).not.toBeNull();
});
it("rebuilds changed content and restores after a virtual row reconnects", () => {
  const view = new LensConversationMarkdown();
  view.markdown = "first";
  document.body.append(view);
  view.markdown = "first **second**";
  expect(view.textContent).toContain("first second");
  view.remove();
  document.body.append(view);
  expect(view.textContent).toContain("first second");
  view.markdown = "replacement";
  expect(view.textContent).toBe("replacement");
});

it("retains exact settled nodes across tab disconnect and reconnect", () => {
  const view = new LensConversationMarkdown();
  view.markdown = "# Retained\n\n**same nodes**";
  document.body.append(view);
  const heading = view.querySelector("h1");
  const bold = view.querySelector("strong");
  view.remove();
  expect(view.querySelector("h1")).toBe(heading);
  document.body.append(view);
  expect(view.querySelector("h1")).toBe(heading);
  expect(view.querySelector("strong")).toBe(bold);
  view.remove();
  view.markdown = "# Changed";
  document.body.append(view);
  expect(view.querySelector("h1")).not.toBe(heading);
  expect(view.textContent).toBe("Changed");
});
