/** Each selector option represents a state the corresponding fixture renders. */
export const PREVIEW_SCENARIOS = {
  about: ["ready", "loading", "failed"],
  settings: ["ready", "loading", "failed", "pending"],
  "target-selection": ["ready", "loading", "failed", "pending"],
  overlay: ["ready", "loading", "failed", "pending", "delayed"],
  "settings-recovery": ["ready", "loading", "failed"],
} as const;
export type PreviewView = keyof typeof PREVIEW_SCENARIOS;
export type PreviewScenario = (typeof PREVIEW_SCENARIOS)[PreviewView][number];
export const PREVIEW_VIEWS = Object.keys(PREVIEW_SCENARIOS) as PreviewView[];
