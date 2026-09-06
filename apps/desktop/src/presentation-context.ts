import { APP_VIEWS, type AppView } from "./page-entries";
export { APP_VIEWS, type AppView };

export const DESKTOP_PLATFORMS = ["macos", "windows", "linux"] as const;
export type DesktopPlatform = (typeof DESKTOP_PLATFORMS)[number];

export interface PresentationContext {
  view: AppView;
  platform: DesktopPlatform;
}

function parseFiniteValue<const T extends readonly string[]>(
  name: string,
  value: string | null,
  allowedValues: T,
): T[number] {
  if (value !== null && allowedValues.includes(value)) return value;

  const received = value === null ? "missing" : JSON.stringify(value);
  throw new Error(
    `Invalid ${name} presentation state: received ${received}; expected ${allowedValues.join(" | ")}`,
  );
}

export function platformFromSearch(search: string): DesktopPlatform {
  return parseFiniteValue(
    "platform",
    new URLSearchParams(search).get("platform"),
    DESKTOP_PLATFORMS,
  );
}

export function presentationContextForPage(
  view: AppView,
  documentView: string | undefined,
  search: string,
): PresentationContext {
  if (documentView !== view)
    throw new Error(`Page identity mismatch: expected ${view}, received ${documentView}`);
  if (new URLSearchParams(search).has("view"))
    throw new Error("Query-based view routing is not supported");
  return { view, platform: platformFromSearch(search) };
}

export function applyPresentationContext(
  context: PresentationContext,
  elements: readonly HTMLElement[],
): void {
  for (const element of elements) {
    element.dataset.view = context.view;
    element.dataset.platform = context.platform;
  }
}
