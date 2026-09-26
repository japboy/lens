import type { PresentationContext } from "./presentation-context";

declare global {
  interface Window {
    __LENS_FLOATING_WINDOW_GEOMETRY__?: unknown;
  }
}

/** Immutable constructor geometry is independent of native color/appearance updates. */
export function installFloatingWindowGeometry(
  context: PresentationContext,
  root: HTMLElement,
  value: unknown = window.__LENS_FLOATING_WINDOW_GEOMETRY__,
): void {
  root.style.removeProperty("--floating-window-corner-radius");
  if (context.view !== "overlay" && context.view !== "target-selection") return;
  if (typeof value !== "object" || value === null) return;
  const radius = (value as Record<string, unknown>).corner_radius;
  if (typeof radius !== "number" || !Number.isFinite(radius) || radius < 0 || radius > 64) return;
  root.style.setProperty("--floating-window-corner-radius", `${radius}px`);
}
