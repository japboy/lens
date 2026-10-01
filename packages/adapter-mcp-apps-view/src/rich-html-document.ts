import { parse, parseFragment, type DefaultTreeAdapterMap } from "parse5";
import { MATH_LIMITS, readMathSpan, renderMathMarkup } from "adapter-math-renderer";
import linkScript from "./assets/rich-html-links.js?raw";

type Node = DefaultTreeAdapterMap["node"];
type Element = DefaultTreeAdapterMap["element"];
type Edit = { start: number; end: number; text: string };
const EXCLUDED = new Set([
  "pre",
  "code",
  "style",
  "script",
  "textarea",
  "template",
  "svg",
  "math",
  "noscript",
  "title",
  "select",
  "option",
  "xmp",
  "noembed",
  "noframes",
  "plaintext",
]);
const HTML_NAMESPACE = "http://www.w3.org/1999/xhtml";
export const MAX_HTML_SOURCE_BYTES = 512 * 1024;
export const MAX_RICH_HTML_DOCUMENT_BYTES = 4 * 1024 * 1024;
const MAX_MATH_BYTES = 2 * 1024 * 1024;
const bytes = (text: string) => new TextEncoder().encode(text).byteLength;
const element = (node: Node): node is Element => "tagName" in node;
const escapeText = (text: string) =>
  text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
function escaped(text: string, at: number): boolean {
  let count = 0;
  while (at > 0 && text[--at] === "\\") count++;
  return count % 2 === 1;
}

/** Built-in presentation derivative only: source locations preserve executable
 * author bytes and natural parser semantics. Raw artifacts/results remain immutable. */
export function prepareRichHtmlDocument(source: string, htmlMathInlineCss: string): string {
  if (bytes(source) > MAX_HTML_SOURCE_BYTES) throw new Error("HTML content exceeds 512 KiB");
  return enhanceHtmlDocument(source, htmlMathInlineCss);
}

/** Serialized static derivatives use the document budget, after the source limit. */
export function prepareSanitizedHtmlDocument(source: string, htmlMathInlineCss: string): string {
  if (bytes(source) > MAX_RICH_HTML_DOCUMENT_BYTES)
    throw new Error("Prepared HTML exceeds the App document budget");
  return enhanceHtmlDocument(source, htmlMathInlineCss);
}

function enhanceHtmlDocument(source: string, htmlMathInlineCss: string): string {
  const document = parse(source, { sourceCodeLocationInfo: true, scriptingEnabled: true });
  const html = document.childNodes.find(
    (node): node is Element => element(node) && node.tagName === "html",
  )!;
  const head = html.childNodes.find(
    (node): node is Element => element(node) && node.tagName === "head",
  )!;
  const body = html.childNodes.find(
    (node): node is Element => element(node) && node.tagName === "body",
  )!;
  const edits: Edit[] = [];
  const pending: Node[] = [...(body?.childNodes ?? [])].reverse();
  let count = 0,
    characters = 0,
    mathBytes = 0,
    outputLimit = false;
  while (pending.length) {
    const node = pending.pop()!;
    if (element(node)) {
      const classes =
        node.attrs.find((attribute) => attribute.name === "class")?.value.split(/\s+/u) ?? [];
      if (
        node.namespaceURI !== HTML_NAMESPACE ||
        EXCLUDED.has(node.tagName) ||
        classes.some((name) => name === "katex" || name.startsWith("katex-"))
      )
        continue;
      pending.push(...[...node.childNodes].reverse());
      continue;
    }
    if (node.nodeName !== "#text" || !("value" in node) || !node.sourceCodeLocation) continue;
    const location = node.sourceCodeLocation;
    // Foster parenting can merge noncontiguous text. Never replace a source range
    // that contains markup or no longer represents this exact text node.
    const original = parseFragment(
      source.slice(location.startOffset, location.endOffset),
    ).childNodes;
    if (
      original.length !== 1 ||
      original[0]?.nodeName !== "#text" ||
      !("value" in original[0]) ||
      original[0].value !== node.value
    )
      continue;
    const text = node.value;
    const openings = /\\[([]|\$\$/gu;
    let match: RegExpExecArray | null;
    let retained = 0,
      replacement = "";
    while ((match = openings.exec(text))) {
      if (escaped(text, match.index)) continue;
      const math = readMathSpan(text.slice(match.index));
      if (!math) break;
      openings.lastIndex = match.index + math.raw.length;
      count++;
      characters += math.tex.length;
      if (
        count > MATH_LIMITS.count ||
        math.tex.length > MATH_LIMITS.characters ||
        characters > MATH_LIMITS.totalCharacters
      )
        continue;
      let markup;
      try {
        markup = renderMathMarkup(math);
      } catch {
        continue;
      }
      mathBytes += bytes(markup);
      if (mathBytes > MAX_MATH_BYTES) {
        outputLimit = true;
        break;
      }
      replacement +=
        escapeText(text.slice(retained, match.index)) +
        `<span class="lens-html-math">${markup}</span>`;
      retained = openings.lastIndex;
    }
    if (outputLimit) break;
    if (replacement)
      edits.push({
        start: location.startOffset,
        end: location.endOffset,
        text: replacement + escapeText(text.slice(retained)),
      });
  }
  if (outputLimit) edits.length = 0;
  const injection = `${edits.length ? `<style data-lens-math>${htmlMathInlineCss}</style>` : ""}<script data-lens-html-links>${linkScript.replace(/<\/script/giu, "<\\/script")}</script>`;
  const headStart = head.sourceCodeLocation?.startTag?.endOffset;
  if (headStart !== undefined) edits.push({ start: headStart, end: headStart, text: injection });
  else {
    const htmlStart = html.sourceCodeLocation?.startTag?.endOffset;
    const doctype = document.childNodes.find((node) => node.nodeName === "#documentType");
    const start = htmlStart ?? doctype?.sourceCodeLocation?.endOffset ?? 0;
    edits.push({ start, end: start, text: `<head>${injection}</head>` });
  }
  edits.sort((left, right) => right.start - left.start);
  let result = source;
  for (const edit of edits)
    result = result.slice(0, edit.start) + edit.text + result.slice(edit.end);
  if (bytes(result) > MAX_RICH_HTML_DOCUMENT_BYTES)
    throw new Error("Prepared HTML exceeds the App document budget");
  return result;
}
