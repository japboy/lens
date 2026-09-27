export type HtmlOutputContent =
  | { resourceId: string; status: "loading" }
  | { resourceId: string; status: "ready"; content: string }
  | { resourceId: string; status: "failed"; message: string };

export const MAX_HTML_OUTPUT_BYTES = 512 * 1024;
