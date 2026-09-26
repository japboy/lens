import type { PresentationContext } from "./presentation-context";

declare global {
  interface Window {
    __LENS_CONTROL_PALETTE__?: unknown;
  }
}

type Srgb = readonly [number, number, number, number];
interface PaletteColors {
  control_surface: Srgb;
  window_surface: Srgb;
  button_fill: Srgb;
  button_pressed_fill: Srgb;
  primary_button_fill: Srgb;
  primary_button_foreground: Srgb;
  separator: Srgb;
}
interface ControlPalette {
  colors: PaletteColors | null;
  increase_contrast: boolean;
  reduce_transparency: boolean;
  window_active: boolean;
}

const attachments = new WeakMap<HTMLElement, () => void>();
const colorProperties = {
  control_surface: "--control-background",
  window_surface: "--window-background",
  button_fill: "--native-button-fill",
  button_pressed_fill: "--native-button-pressed-fill",
  primary_button_fill: "--native-primary-button-fill",
  primary_button_foreground: "--native-primary-button-foreground",
  separator: "--native-separator",
} as const;

function isSrgb(value: unknown, opaque = false): value is Srgb {
  return (
    Array.isArray(value) &&
    value.length === 4 &&
    [...value].every((channel) => Number.isInteger(channel) && channel >= 0 && channel <= 255) &&
    (!opaque || value[3] === 255)
  );
}

function parsePalette(value: unknown): ControlPalette | undefined {
  if (typeof value !== "object" || value === null) return undefined;
  const palette = value as Record<string, unknown>;
  if (
    typeof palette.increase_contrast !== "boolean" ||
    typeof palette.reduce_transparency !== "boolean" ||
    typeof palette.window_active !== "boolean"
  )
    return undefined;
  const colors = palette.colors;
  if (colors !== null) {
    if (typeof colors !== "object" || colors === undefined) return undefined;
    const entries = colors as Record<string, unknown>;
    if (
      !Object.keys(colorProperties).every((key) =>
        isSrgb(
          entries[key],
          [
            "control_surface",
            "window_surface",
            "primary_button_fill",
            "primary_button_foreground",
          ].includes(key),
        ),
      )
    )
      return undefined;
  }
  return {
    colors: colors as PaletteColors | null,
    increase_contrast: palette.increase_contrast,
    reduce_transparency: palette.reduce_transparency,
    window_active: palette.window_active,
  };
}

function cssColor([red, green, blue, alpha]: Srgb): string {
  return alpha === 255
    ? `rgb(${red} ${green} ${blue})`
    : `rgb(${red} ${green} ${blue} / ${alpha / 255})`;
}

/** Apply one complete native snapshot before platform styling and DSD attachment. */
export function installControlPalette(
  context: PresentationContext,
  root: HTMLElement,
  view: HTMLElement | null,
  source: Window = window,
): void {
  attachments.get(root)?.();
  const boundaries = [root, ...(view ? [view] : [])];
  const apply = (value: unknown): void => {
    const palette = parsePalette(value);
    for (const [key, property] of Object.entries(colorProperties)) {
      const color = palette?.colors?.[key as keyof PaletteColors];
      if (color) root.style.setProperty(property, cssColor(color));
      else root.style.removeProperty(property);
    }
    for (const boundary of boundaries) {
      if (palette) {
        boundary.dataset.increaseContrast = String(palette.increase_contrast);
        boundary.dataset.reduceTransparency = String(palette.reduce_transparency);
        boundary.dataset.windowActive = String(palette.window_active);
        boundary.dataset.nativeControls = String(palette.colors !== null);
      } else {
        delete boundary.dataset.increaseContrast;
        delete boundary.dataset.reduceTransparency;
        delete boundary.dataset.windowActive;
        delete boundary.dataset.nativeControls;
      }
    }
  };
  if (context.platform !== "macos") {
    apply(null);
    return;
  }

  const changed = (event: Event): void => {
    apply(event instanceof CustomEvent ? event.detail : null);
  };
  const detach = (): void => {
    source.removeEventListener("lens-control-palette", changed);
    source.removeEventListener("pagehide", pageHidden);
    attachments.delete(root);
  };
  const pageHidden = (event: PageTransitionEvent): void => {
    if (!event.persisted) detach();
  };
  source.addEventListener("lens-control-palette", changed);
  source.addEventListener("pagehide", pageHidden);
  attachments.set(root, detach);
  apply(source.__LENS_CONTROL_PALETTE__);
}
