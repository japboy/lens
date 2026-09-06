/** Every production WebView has an independent document and module entry. */
export const PAGE_ENTRIES = {
  about: "about.html",
  settings: "settings.html",
  overlay: "overlay.html",
  "target-selection": "target-selection.html",
} as const;
export type AppView = keyof typeof PAGE_ENTRIES;
export const APP_VIEWS = Object.keys(PAGE_ENTRIES) as AppView[];
