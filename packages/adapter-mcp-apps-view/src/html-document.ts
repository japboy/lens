import { parse, serialize, type DefaultTreeAdapterMap } from "parse5";
import {
  prepareRichHtmlDocument,
  prepareSanitizedHtmlDocument,
  MAX_HTML_SOURCE_BYTES,
} from "./rich-html-document";

export type HtmlDocumentMode = "static" | "interactive";
type Node = DefaultTreeAdapterMap["node"];
type Element = DefaultTreeAdapterMap["element"];
const isElement = (node: Node): node is Element => "tagName" in node;

function safeHtmlLink(value: string): string | undefined {
  try {
    const url = new URL(value);
    return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password
      ? url.href
      : undefined;
  } catch {
    return undefined;
  }
}

/** One document pipeline. Static author code is removed before the same trusted
 * math/link enhancement; native mode-specific CSP independently prevents execution. */
export function prepareHtmlDocument(
  source: string,
  mode: HtmlDocumentMode,
  inlineMathCss: string,
): string {
  if (new TextEncoder().encode(source).byteLength > MAX_HTML_SOURCE_BYTES)
    throw new Error("HTML content exceeds 512 KiB");
  if (mode === "interactive") return prepareRichHtmlDocument(source, inlineMathCss);
  const document = parse(source, { scriptingEnabled: false });
  const pending: Node[] = [document];
  const removedTags = new Set([
    "script",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "base",
    "link",
  ]);
  while (pending.length) {
    const node = pending.pop()!;
    if (!isElement(node)) {
      if ("childNodes" in node) pending.push(...[...node.childNodes].reverse());
      continue;
    }
    const tag = node.tagName.toLowerCase();
    // SVG SMIL can rewrite href without JavaScript. Retain visual animation,
    // but do not let it restore navigation or resource attributes.
    const animatedAttribute = node.attrs
      .find((attr) => attr.name.toLowerCase() === "attributename")
      ?.value.toLowerCase();
    const changesUrl =
      ["animate", "set"].includes(tag) &&
      !!animatedAttribute &&
      /(?:href|src|action|target|ping|download)$/.test(animatedAttribute);
    if (
      removedTags.has(tag) ||
      changesUrl ||
      (tag === "meta" && node.attrs.some((attr) => attr.name === "http-equiv"))
    ) {
      if (node.parentNode)
        node.parentNode.childNodes = node.parentNode.childNodes.filter((child) => child !== node);
      continue;
    }
    const anchor = tag === "a" || tag === "area";
    node.attrs = node.attrs.filter((attr) => {
      const name = attr.name.toLowerCase();
      if (
        name.startsWith("on") ||
        ["action", "formaction", "target", "formtarget", "ping", "download"].includes(name)
      )
        return false;
      if (anchor && name === "href" && !attr.value.trim().startsWith("#")) {
        const url = safeHtmlLink(attr.value);
        if (!url) return false;
        attr.value = url;
      }
      return true;
    });
    pending.push(...[...node.childNodes].reverse());
    if ("content" in node) pending.push((node as DefaultTreeAdapterMap["template"]).content);
  }
  return prepareSanitizedHtmlDocument(
    serialize(document, { scriptingEnabled: false }),
    inlineMathCss,
  );
}
