import DOMPurify, { type Config } from "dompurify";
import { Marked } from "marked";

const markdownParser = new Marked({
  async: false,
  breaks: false,
  gfm: true,
  pedantic: false,
});

const SANITIZE_OPTIONS: Config = {
  USE_PROFILES: { html: true },
  ALLOW_DATA_ATTR: false,
  FORBID_TAGS: ["button", "form", "iframe", "object", "embed", "select", "style", "textarea"],
  FORBID_ATTR: ["style"],
};

export function renderMarkdown(markdown: string): string {
  const parsed = markdownParser.parse(markdown, { async: false });
  return DOMPurify.sanitize(parsed, SANITIZE_OPTIONS);
}

export function externalMarkdownUrl(href: string): string | undefined {
  try {
    const url = new URL(href);
    return url.protocol === "https:" || url.protocol === "http:" || url.protocol === "mailto:"
      ? url.href
      : undefined;
  } catch {
    return undefined;
  }
}
