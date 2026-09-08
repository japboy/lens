import DOMPurify from "dompurify";

export const MAX_HTML_BYTES = 512 * 1024;
const sanitizer = DOMPurify(window);
const tags = [
  "html",
  "head",
  "body",
  "style",
  "div",
  "section",
  "article",
  "header",
  "footer",
  "main",
  "aside",
  "span",
  "p",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "ul",
  "ol",
  "li",
  "dl",
  "dt",
  "dd",
  "table",
  "caption",
  "thead",
  "tbody",
  "tfoot",
  "tr",
  "th",
  "td",
  "strong",
  "b",
  "em",
  "i",
  "u",
  "s",
  "small",
  "sub",
  "sup",
  "pre",
  "code",
  "blockquote",
  "br",
  "hr",
  "a",
];
const length = /^(?:0|(?:\d{1,3}(?:\.\d{1,2})?)(?:px|em|rem|%))$/;
const color =
  /^(?:#[\da-f]{3,8}|black|white|gray|grey|red|green|blue|yellow|orange|purple|transparent|currentcolor)$/i;
const enums: Record<string, readonly string[]> = {
  display: [
    "block",
    "inline",
    "inline-block",
    "flex",
    "grid",
    "table",
    "table-row",
    "table-cell",
    "none",
  ],
  "flex-direction": ["row", "column", "row-reverse", "column-reverse"],
  "flex-wrap": ["wrap", "nowrap"],
  "justify-content": [
    "start",
    "end",
    "center",
    "space-between",
    "space-around",
    "flex-start",
    "flex-end",
  ],
  "align-items": ["start", "end", "center", "stretch", "baseline", "flex-start", "flex-end"],
  "text-align": ["left", "right", "center", "start", "end"],
  "font-weight": ["normal", "bold", "100", "200", "300", "400", "500", "600", "700", "800", "900"],
  "font-style": ["normal", "italic"],
  "font-family": ["serif", "sans-serif", "monospace", "system-ui"],
  "white-space": ["normal", "pre-wrap", "pre-line"],
  "overflow-wrap": ["normal", "break-word", "anywhere"],
  "border-collapse": ["collapse", "separate"],
  "border-style": ["solid", "dashed", "dotted", "none"],
  "table-layout": ["auto", "fixed"],
};
const lengths = new Set([
  "padding",
  "padding-top",
  "padding-right",
  "padding-bottom",
  "padding-left",
  "margin",
  "margin-top",
  "margin-right",
  "margin-bottom",
  "margin-left",
  "gap",
  "row-gap",
  "column-gap",
  "border-radius",
  "border-width",
  "font-size",
  "width",
  "max-width",
  "min-width",
]);

/** Finite CSS grammar: no escapes, URLs, custom properties, functions, or positioning. */
function declarations(input: string): string {
  if (/[\\{}@<>]|\/\*/.test(input)) return "";
  return input
    .split(";")
    .slice(0, 128)
    .flatMap((declaration) => {
      const separator = declaration.indexOf(":");
      if (separator < 0 || declaration.indexOf(":", separator + 1) !== -1) return [];
      const property = declaration.slice(0, separator).trim();
      const value = declaration
        .slice(separator + 1)
        .trim()
        .toLowerCase();
      if (!/^[a-z-]+$/.test(property) || !value) return [];
      const parts = value.split(/\s+/);
      const accepted =
        (Object.hasOwn(enums, property!) && enums[property!]!.includes(value)) ||
        (lengths.has(property!) && parts.length <= 4 && parts.every((part) => length.test(part))) ||
        (["color", "background-color", "border-color"].includes(property!) && color.test(value)) ||
        (property === "line-height" && /^(?:[12](?:\.\d{1,2})?|normal)$/.test(value)) ||
        (property === "grid-template-columns" &&
          parts.length <= 8 &&
          parts.every((part) => length.test(part) || /^[1-9]fr$/.test(part)));
      return accepted ? [`${property}:${value}`] : [];
    })
    .join(";");
}

function stylesheet(input: string): string {
  if (/[\\@<>]|\/\*/.test(input)) return "";
  // Consume each character once, including malformed input with no opening brace.
  // An unanchored rule regex rescans every suffix of such a stylesheet.
  const rules: Array<[string, string]> = [];
  let start = 0;
  let opening: number | undefined;
  for (let cursor = 0; cursor < input.length; cursor += 1) {
    const character = input[cursor];
    if (character === "{") {
      if (opening !== undefined || cursor === start) return "";
      opening = cursor;
    } else if (character === "}") {
      if (opening === undefined || rules.length === 256) return "";
      rules.push([input.slice(start, opening), input.slice(opening + 1, cursor)]);
      opening = undefined;
      start = cursor + 1;
    }
  }
  if (opening !== undefined || input.slice(start).trim()) return "";
  return rules
    .flatMap(([selectors, body]) => {
      // At most 16 alternatives, each with 512 characters and 16 tag/class tokens.
      // Only simple selectors and descendant/child combinations are accepted.
      const choices = selectors!
        .trim()
        .split(",")
        .map((selector) => selector.trim().replace(/^(?:html|body|:root)$/, ".lens-document-body"));
      if (
        choices.length > 16 ||
        !choices.every(
          (s) =>
            s.length <= 512 &&
            s.split(/[\s>]+/).length <= 16 &&
            /^(?:[a-z][a-z0-9-]*|\.[a-zA-Z_][\w-]*)(?:(?:\s+|\s*>\s*)(?:[a-z][a-z0-9-]*|\.[a-zA-Z_][\w-]*))*$/.test(
              s,
            ),
        )
      )
        return [];
      const safe = declarations(body!);
      return safe ? [`${choices.map((s) => `.content ${s.trim()}`).join(",")}{${safe}}`] : [];
    })
    .join("\n");
}

export function safeHtmlLink(value: string): string | undefined {
  try {
    const url = new URL(value);
    return ["https:", "http:"].includes(url.protocol) ? url.href : undefined;
  } catch {
    return undefined;
  }
}

/** Parses in DOMPurify's inert document; only sanitized nodes enter the live ShadowRoot. */
export function renderStaticHtml(content: string): DocumentFragment {
  if (new TextEncoder().encode(content).byteLength > MAX_HTML_BYTES)
    throw new Error("HTML exceeds the 512 KiB display limit.");
  const sanitized = sanitizer.sanitize(content, {
    WHOLE_DOCUMENT: true,
    RETURN_DOM: true,
    ALLOWED_TAGS: tags,
    ALLOWED_ATTR: ["class", "style", "title", "href", "colspan", "rowspan", "scope", "lang", "dir"],
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
    FORBID_TAGS: ["svg", "math", "template"],
  });
  if (!(sanitized instanceof Element)) throw new Error("HTML document could not be parsed.");
  const document = sanitized;
  let count = 0;
  const visit = (node: Node, depth: number): void => {
    if (++count > 5000 || depth > 64)
      throw new Error("HTML exceeds the document complexity limit.");
    for (const child of node.childNodes) visit(child, depth + 1);
  };
  visit(document, 0);
  const styles = [...document.querySelectorAll("style")];
  if (
    styles.length > 256 ||
    styles.reduce((count, style) => count + (style.textContent?.match(/\{/g)?.length ?? 0), 0) > 256
  )
    throw new Error("HTML exceeds the stylesheet limit.");
  const css = styles.map((style) => stylesheet(style.textContent ?? "")).join("\n");
  styles.forEach((style) => style.remove());
  for (const element of document.querySelectorAll("[style]")) {
    const safe = declarations(element.getAttribute("style") ?? "");
    element.removeAttribute("style");
    if (safe) element.setAttribute("style", safe);
  }
  for (const element of document.querySelectorAll("a[href]")) {
    const url = safeHtmlLink(element.getAttribute("href") ?? "");
    element.removeAttribute("href");
    if (url) element.setAttribute("href", url);
  }
  const result = window.document.createDocumentFragment();
  if (css) {
    const style = window.document.createElement("style");
    style.textContent = css;
    result.append(style);
  }
  const body = document.querySelector("body");
  if (!body?.textContent?.trim()) throw new Error("HTML contains no displayable text.");
  const wrapper = window.document.createElement("div");
  for (const attribute of body.attributes) wrapper.setAttribute(attribute.name, attribute.value);
  wrapper.classList.add("lens-document-body");
  wrapper.append(...body.childNodes);
  result.append(wrapper);
  return result;
}
