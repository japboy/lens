/// Parses a URL and keeps it only when its scheme is explicitly allowed.
///
/// The allowed set is a security boundary, so it is passed in rather than baked in: the
/// markdown renderer and the HTML preview sanitizer allow deliberately different schemes,
/// and that difference belongs at the call sites instead of in two parsers that can drift
/// apart unnoticed.
export function allowedUrl(value: string, schemes: readonly string[]): string | undefined {
  try {
    const url = new URL(value);
    return schemes.includes(url.protocol) ? url.href : undefined;
  } catch {
    return undefined;
  }
}

/// Schemes a rendered document may navigate to.
export const WEB_SCHEMES = ["https:", "http:"] as const;

/// Prose may also address a person; generated HTML previews may not.
export const WEB_AND_MAIL_SCHEMES = [...WEB_SCHEMES, "mailto:"] as const;
