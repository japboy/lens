import { htmlMathPolicy, htmlMathResources } from "./styles/html-math-resources";
import katex from "katex";
import { parseFragment, serialize, type DefaultTreeAdapterMap } from "parse5";
import { MATH_LIMITS, readMathSpan } from "./markdown-math";

type Node = DefaultTreeAdapterMap["node"];
type Element = DefaultTreeAdapterMap["element"];
type TextNode = DefaultTreeAdapterMap["textNode"];
const HTML_NAMESPACE = "http://www.w3.org/1999/xhtml";
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
export const MAX_MATH_PREVIEW_BYTES = 2 * 1024 * 1024;
export interface HtmlMathResult {
  document: string;
  status: "none" | "rendered" | "output-limit";
}
function element(node: Node): node is Element {
  return "tagName" in node;
}
function textNode(value: string): TextNode {
  return { nodeName: "#text", value, parentNode: null };
}
function escaped(source: string, at: number): boolean {
  let count = 0;
  while (at > 0 && source[--at] === "\\") count++;
  return count % 2 === 1;
}

/** Enhance an already sanitized inert document. The original serialization is the
 * atomic fallback when generated math and its local fonts exceed the output budget. */
export function renderHtmlMath(document: DefaultTreeAdapterMap["document"]): HtmlMathResult {
  const baseline = serialize(document, { scriptingEnabled: false });
  const fallback = (status: "none" | "output-limit"): HtmlMathResult => ({
    document: baseline,
    status,
  });
  const html = document.childNodes.find(
    (node): node is Element => element(node) && node.tagName === "html",
  )!;
  const head = html.childNodes.find(
    (node): node is Element => element(node) && node.tagName === "head",
  )!;
  const body = html.childNodes.find(
    (node): node is Element => element(node) && node.tagName === "body",
  )!;
  const pending: Node[] = [...body.childNodes].reverse();
  let count = 0;
  let characters = 0;
  let renderedCount = 0;
  let generatedBytes = 0;
  const byteLength = (source: string) => new TextEncoder().encode(source).byteLength;
  const budget = MAX_MATH_PREVIEW_BYTES;
  while (pending.length) {
    const node = pending.pop()!;
    if (element(node)) {
      const classes = node.attrs.find((attr) => attr.name === "class")?.value.split(/\s+/u) ?? [];
      if (
        node.namespaceURI !== HTML_NAMESPACE ||
        EXCLUDED.has(node.tagName.toLowerCase()) ||
        classes.some((name) => name === "katex" || name.startsWith("katex-"))
      )
        continue;
      pending.push(...[...node.childNodes].reverse());
      continue;
    }
    if (node.nodeName !== "#text" || !node.parentNode) continue;
    const source = (node as TextNode).value;
    const replacement: DefaultTreeAdapterMap["childNode"][] = [];
    const openings = /\\[([]|\$\$/gu;
    let match: RegExpExecArray | null;
    let retainedFrom = 0;
    while ((match = openings.exec(source))) {
      if (escaped(source, match.index)) continue;
      const math = readMathSpan(source.slice(match.index));
      // Incomplete expressions are literal, including their remaining text. Never
      // join delimiters across siblings or reinterpret the inside of unfinished TeX.
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
      let markup: string;
      try {
        markup = katex.renderToString(math.tex, {
          displayMode: math.displayMode,
          output: "htmlAndMathml",
          trust: false,
          strict: "error",
          throwOnError: true,
          maxExpand: 1000,
          maxSize: 20,
          macros: {},
        });
      } catch {
        continue;
      }
      generatedBytes += byteLength(markup);
      if (generatedBytes > budget) return fallback("output-limit");
      replacement.push(textNode(source.slice(retainedFrom, match.index)));
      // Only the locally generated KaTeX string enters this parser. User text is
      // represented by AST text nodes, never interpolated into generated markup.
      replacement.push(
        ...parseFragment(`<span class="lens-html-math">${markup}</span>`, {
          scriptingEnabled: false,
        }).childNodes,
      );
      retainedFrom = openings.lastIndex;
      renderedCount++;
    }
    if (replacement.length) {
      replacement.push(textNode(source.slice(retainedFrom)));
      const parent = node.parentNode;
      const at = parent.childNodes.indexOf(node);
      for (const child of replacement) child.parentNode = parent;
      parent.childNodes.splice(at, 1, ...replacement);
    }
  }
  if (!renderedCount) return fallback("none");
  const resources = htmlMathResources();
  const policy = head.childNodes.find(
    (node): node is Element =>
      element(node) &&
      node.tagName === "meta" &&
      node.attrs.some(
        (attribute) =>
          attribute.name === "http-equiv" && attribute.value === "Content-Security-Policy",
      ),
  )!;
  const content = policy.attrs.find((attribute) => attribute.name === "content")!;
  content.value = htmlMathPolicy(content.value, resources);
  const stylesheet = parseFragment('<link rel="stylesheet">').childNodes[0] as Element;
  stylesheet.attrs.push({ name: "href", value: resources.stylesheet });
  stylesheet.parentNode = head;
  head.childNodes.push(stylesheet);
  const renderedDocument = serialize(document, { scriptingEnabled: false });
  if (byteLength(renderedDocument) > budget) return fallback("output-limit");
  return { document: renderedDocument, status: "rendered" };
}
