import DOMPurify, { type Config } from "dompurify";
import { Marked } from "marked";
import { allowedUrl, WEB_AND_MAIL_SCHEMES } from "./external-url";

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

const HTML_NAMESPACE = "http://www.w3.org/1999/xhtml";
const SVG_NAMESPACE = "http://www.w3.org/2000/svg";
const MAX_MERMAID_LABEL_LAYOUT_SIZE_PX = 10_000;
export const MERMAID_HTML_LABEL_TAGS = [
  "div",
  "span",
  "p",
  "b",
  "strong",
  "i",
  "em",
  "u",
  "s",
  "del",
  "code",
  "sub",
  "sup",
  "br",
] as const;
const MERMAID_HTML_LABEL_TAG_SET: ReadonlySet<string> = new Set(MERMAID_HTML_LABEL_TAGS);
const MERMAID_LABEL_DIV_CLASSES: ReadonlySet<string> = new Set(["labelBkg"]);
const MERMAID_LABEL_SPAN_CLASSES: ReadonlySet<string> = new Set([
  "nodeLabel",
  "edgeLabel",
  "markdown-node-label",
]);
const MERMAID_LABEL_LAYOUT_PROPERTIES = [
  "display",
  "white-space",
  "line-height",
  "max-width",
  "width",
  "text-align",
] as const;
type MermaidLabelLayoutProperty = (typeof MERMAID_LABEL_LAYOUT_PROPERTIES)[number];
type MermaidHtmlLabelRole = "content" | "label" | "layout";

const MERMAID_SVG_SANITIZE_OPTIONS: Config = {
  USE_PROFILES: { svg: true, svgFilters: true },
  ADD_TAGS: ["foreignObject", ...MERMAID_HTML_LABEL_TAGS],
  ADD_ATTR: ["class", "style", "xmlns"],
  ALLOW_DATA_ATTR: false,
  FORBID_TAGS: ["a", "script"],
  HTML_INTEGRATION_POINTS: { foreignobject: true },
};

const mermaidSvgSanitizer = DOMPurify(window);

function isForeignObject(element: Element | null): boolean {
  return (
    element?.namespaceURI === SVG_NAMESPACE && element.localName.toLowerCase() === "foreignobject"
  );
}

function hasForeignObjectAncestor(element: Element): boolean {
  for (let ancestor = element.parentElement; ancestor !== null; ancestor = ancestor.parentElement) {
    if (isForeignObject(ancestor)) return true;
  }
  return false;
}

function mermaidHtmlLabelRole(element: Element): MermaidHtmlLabelRole {
  if (element.localName.toLowerCase() === "div" && isForeignObject(element.parentElement)) {
    return "layout";
  }
  if (
    element.localName.toLowerCase() === "span" &&
    element.parentElement?.namespaceURI === HTML_NAMESPACE &&
    mermaidHtmlLabelRole(element.parentElement) === "layout"
  ) {
    return "label";
  }
  return "content";
}

function sanitizeMermaidLabelClasses(value: string, allowed: ReadonlySet<string>): string {
  return value
    .split(/\s+/u)
    .filter((className) => allowed.has(className))
    .join(" ");
}

function isSafeMermaidPixelLength(value: string): boolean {
  if (!/^(?:0|[1-9]\d{0,4})(?:\.\d+)?px$/u.test(value)) return false;
  return Number.parseFloat(value) <= MAX_MERMAID_LABEL_LAYOUT_SIZE_PX;
}

function isAllowedMermaidLabelLayoutValue(
  property: MermaidLabelLayoutProperty,
  value: string,
): boolean {
  switch (property) {
    case "display":
      return value === "table" || value === "table-cell";
    case "white-space":
      return value === "break-spaces" || value === "nowrap";
    case "line-height":
      return value === "1.5";
    case "max-width":
    case "width":
      return isSafeMermaidPixelLength(value);
    case "text-align":
      return value === "center";
  }
}

function sanitizeMermaidLabelLayoutStyle(value: string): string {
  const declarations = new Map<MermaidLabelLayoutProperty, string>();

  for (const declaration of value.split(";")) {
    const colon = declaration.indexOf(":");
    if (colon < 0) continue;
    const property = declaration.slice(0, colon).trim().toLowerCase();
    if (!MERMAID_LABEL_LAYOUT_PROPERTIES.includes(property as MermaidLabelLayoutProperty)) {
      continue;
    }
    const typedProperty = property as MermaidLabelLayoutProperty;
    const propertyValue = declaration
      .slice(colon + 1)
      .trim()
      .toLowerCase();
    if (isAllowedMermaidLabelLayoutValue(typedProperty, propertyValue)) {
      declarations.set(typedProperty, propertyValue);
    }
  }

  return MERMAID_LABEL_LAYOUT_PROPERTIES.filter((property) => declarations.has(property))
    .map((property) => `${property}: ${declarations.get(property)}`)
    .join("; ");
}

mermaidSvgSanitizer.addHook("uponSanitizeElement", (node) => {
  if (!(node instanceof Element) || !hasForeignObjectAncestor(node)) return;
  if (
    node.namespaceURI !== HTML_NAMESPACE ||
    !MERMAID_HTML_LABEL_TAG_SET.has(node.localName.toLowerCase())
  ) {
    node.remove();
  }
});

mermaidSvgSanitizer.addHook("uponSanitizeAttribute", (element, event) => {
  if (element.namespaceURI === HTML_NAMESPACE) {
    const role = mermaidHtmlLabelRole(element);
    if (event.attrName === "xmlns" && role === "layout" && event.attrValue === HTML_NAMESPACE) {
      return;
    }
    if (event.attrName === "class") {
      const allowedClasses =
        role === "layout"
          ? MERMAID_LABEL_DIV_CLASSES
          : role === "label"
            ? MERMAID_LABEL_SPAN_CLASSES
            : undefined;
      const classes =
        allowedClasses === undefined
          ? ""
          : sanitizeMermaidLabelClasses(event.attrValue, allowedClasses);
      if (classes.length > 0) {
        event.attrValue = classes;
        return;
      }
    }
    if (event.attrName === "style" && role === "layout") {
      const style = sanitizeMermaidLabelLayoutStyle(event.attrValue);
      if (style.length > 0) {
        event.attrValue = style;
        return;
      }
    }
    event.keepAttr = false;
    return;
  }

  if (
    isForeignObject(element) &&
    event.attrName !== "x" &&
    event.attrName !== "y" &&
    event.attrName !== "width" &&
    event.attrName !== "height"
  ) {
    event.keepAttr = false;
  }
});

export function renderMarkdown(markdown: string): string {
  const parsed = markdownParser.parse(markdown, { async: false });
  return DOMPurify.sanitize(parsed, SANITIZE_OPTIONS);
}

export function sanitizeMermaidSvg(svg: string): DocumentFragment {
  return mermaidSvgSanitizer.sanitize(svg, {
    ...MERMAID_SVG_SANITIZE_OPTIONS,
    RETURN_DOM_FRAGMENT: true,
  });
}

export function externalMarkdownUrl(href: string): string | undefined {
  return allowedUrl(href, WEB_AND_MAIL_SCHEMES);
}
