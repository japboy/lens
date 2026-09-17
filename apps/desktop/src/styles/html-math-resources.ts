import { htmlMathAssets } from "virtual:lens-html-math-assets";

export interface HtmlMathResources {
  readonly stylesheet: string;
  readonly fonts: readonly string[];
}

/** Resolve trusted build output, never an author-provided URL. HTTP development
 * keeps its immutable generation; packaged WebViews use the public math protocol. */
export function htmlMathResources(
  documentUrl = globalThis.location?.href ?? "tauri://localhost/",
  buildBase = import.meta.env.BASE_URL,
): HtmlMathResources {
  const document = new URL(documentUrl);
  const native = document.protocol === "tauri:" || document.hostname === "tauri.localhost";
  const base = native
    ? document.protocol === "tauri:"
      ? "lens-math://assets/"
      : `${document.protocol}//lens-math.assets/`
    : new URL(buildBase, document).href;
  const url = (path: string) => new URL(path, base).href;
  return {
    stylesheet: url(htmlMathAssets.stylesheetPath),
    fonts: htmlMathAssets.fontPaths.map(url),
  };
}

export function htmlMathPolicy(policy: string, resources: HtmlMathResources): string {
  return policy
    .replace("style-src 'unsafe-inline'", `style-src 'unsafe-inline' ${resources.stylesheet}`)
    .replace("font-src data:", `font-src data: ${resources.fonts.join(" ")}`);
}
