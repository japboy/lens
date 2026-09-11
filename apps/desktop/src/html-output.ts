import { parse, serialize, type DefaultTreeAdapterMap } from "parse5";
import { allowedUrl, WEB_SCHEMES } from "./external-url";

export const MAX_HTML_BYTES = 512 * 1024;

// Supplements the iframe sandbox. Only popup requests are permitted; Lens's
// native new-window handler cancels them and opens HTTP(S) in the browser.
export const HTML_PREVIEW_CSP = [
  "default-src 'none'",
  "script-src 'none'",
  "style-src 'unsafe-inline'",
  "img-src data:",
  "font-src data:",
  "media-src data:",
  "connect-src 'none'",
  "frame-src 'none'",
  "object-src 'none'",
  "form-action 'none'",
  "base-uri about:",
].join("; ");

export interface PreparedHtmlPreview {
  document: string;
  notices: string[];
}

type Node = DefaultTreeAdapterMap["node"];
type Element = DefaultTreeAdapterMap["element"];

export function safeHtmlLink(value: string): string | undefined {
  return allowedUrl(value, WEB_SCHEMES);
}

function isElement(node: Node): node is Element {
  return "tagName" in node;
}

/** Parse without a browsing context so preparation cannot fetch resources.
 * Preserve document structure and CSS; remove active/remote document controls.
 * Mount only in a sandbox="allow-popups" iframe with Lens's native popup handler.
 */
export function prepareHtmlPreview(content: string): PreparedHtmlPreview {
  if (new TextEncoder().encode(content).byteLength > MAX_HTML_BYTES) {
    throw new Error("HTML content exceeds 512 KiB");
  }
  const document = parse(content, { scriptingEnabled: false });
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
    if (anchor && node.attrs.some((attr) => attr.name === "href" && safeHtmlLink(attr.value))) {
      node.attrs = node.attrs.filter((attr) => attr.name !== "rel");
      node.attrs.push(
        { name: "target", value: "_blank" },
        { name: "rel", value: "noopener noreferrer" },
      );
    }
    pending.push(...[...node.childNodes].reverse());
    if ("content" in node) pending.push((node as DefaultTreeAdapterMap["template"]).content);
  }
  const html = document.childNodes.find(
    (node): node is Element => isElement(node) && node.tagName === "html",
  )!;
  const head = html.childNodes.find(
    (node): node is Element => isElement(node) && node.tagName === "head",
  )!;
  const trusted = parse(
    `<head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${HTML_PREVIEW_CSP}"><base href="about:srcdoc"></head>`,
  );
  const trustedHtml = trusted.childNodes.find(isElement)!;
  const trustedHead = trustedHtml.childNodes.find(
    (node): node is Element => isElement(node) && node.tagName === "head",
  )!;
  for (const node of trustedHead.childNodes) node.parentNode = head;
  head.childNodes.unshift(...trustedHead.childNodes);
  return {
    document: serialize(document, { scriptingEnabled: false }),
    notices: [
      "JavaScript, external resources, embedded documents, and form submission are disabled.",
      "HTTP and HTTPS links open in your default browser.",
    ],
  };
}
