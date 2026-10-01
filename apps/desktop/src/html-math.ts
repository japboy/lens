import { renderStaticHtmlMath } from "adapter-math-renderer";
import type { DefaultTreeAdapterMap } from "parse5";
import { htmlMathPolicy, htmlMathResources } from "./styles/html-math-resources";

export { MAX_MATH_PREVIEW_BYTES, type HtmlMathResult } from "adapter-math-renderer";

/** Desktop selects its trusted built resource URLs; the renderer owns the math transform. */
export function renderHtmlMath(document: DefaultTreeAdapterMap["document"]) {
  return renderStaticHtmlMath(document, {
    resources: htmlMathResources,
    applyPolicy: htmlMathPolicy,
  });
}
