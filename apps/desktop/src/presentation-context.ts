export const APP_VIEWS = ["settings", "overlay", "target-selection", "about"] as const;
export type AppView = (typeof APP_VIEWS)[number];

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

export function presentationContextFromSearch(search: string): PresentationContext {
  const parameters = new URLSearchParams(search);
  return {
    view: parseFiniteValue("view", parameters.get("view"), APP_VIEWS),
    platform: parseFiniteValue("platform", parameters.get("platform"), DESKTOP_PLATFORMS),
  };
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
