import { version } from "../package.json";
import "ui/styles/icon-fonts.css";
import "ui/styles/math-fonts.css";
import { previewDocumentLoader } from "./services";
import { PREVIEW_SCENARIOS, PREVIEW_VIEWS } from "./scenarios";
import { DESKTOP_PLATFORMS } from "ui/contracts/context";
import type { LensState } from "ui/contracts/lens";
function finiteValue<const T extends readonly string[]>(value: string, allowed: T): T[number] {
  if (allowed.includes(value)) return value;
  throw new Error(`Unexpected preview state ${value}`);
}
import "ui/entries/about";
import "ui/entries/settings";
import "ui/entries/settings-defaults";
import "ui/entries/settings-prompts";
import "ui/entries/target-selection";
import "ui/entries/overlay-output";
import "ui/entries/overlay-session";
import "ui/entries/overlay-source";
import "ui/entries/overlay-conversation";
import "ui/styles/document.css";
import {
  LensSettingsRecoveryView,
  LensAboutView,
  LensSettingsView,
  LensTargetSelectionView,
  LensOverlayView,
} from "ui";
export const intents: unknown[] = [];
const mount = document.querySelector<HTMLElement>("#mount")!;
const viewControl = document.querySelector<HTMLSelectElement>("#view")!;
const scenarioControl = document.querySelector<HTMLSelectElement>("#scenario")!;
const platformControl = document.querySelector<HTMLSelectElement>("#platform")!;
const template = {
  schema_version: 1 as const,
  common: "Explain {turn_instruction}",
  full_projection: "Read the complete input",
  source_checkpoint: "Replace {base_revision} with {target_revision}",
  current_projection_retry: "Retry {applied_revision}",
};
const lens: LensState = {
  operation_id: "preview-operation",
  stage: "completed" as const,
  prompt_execution_revision: 1,
  response_history: { responses: [], retained_bytes: 0, capacity_reached: false },
  output_blocks: [
    {
      type: "markdown" as const,
      text: "## Actual Lens rendering\n\nA browser-only consumer renders the **same UI** and math: $x^2+y^2=z^2$.\n\n- Models flow down\n- Semantic intents flow up",
    },
  ],
};
const targetLens = {
  ...lens,
  stage: "selecting" as const,
  selection: {
    selection_id: "preview-selection",
    stage: "reviewing" as const,
    maximum_targets: 4,
    items: [
      {
        id: "preview-target",
        window: {
          window_id: 1,
          title: "Independent UI preview",
          application_name: "Browser",
          bundle_id: "preview",
          pid: 1,
          frame: { x: 0, y: 0, width: 960, height: 640 },
        },
      },
    ],
  },
};
function setOptions(control: HTMLSelectElement, values: readonly string[]) {
  const selected = control.value;
  control.replaceChildren(...values.map((value) => new Option(value, value)));
  control.value = values.includes(selected) ? selected : values[0]!;
}
setOptions(viewControl, PREVIEW_VIEWS);
setOptions(platformControl, DESKTOP_PLATFORMS);
export function show() {
  const selected = finiteValue(viewControl.value, PREVIEW_VIEWS);
  const scenarios = PREVIEW_SCENARIOS[selected];
  setOptions(scenarioControl, scenarios);
  const scenario = finiteValue(scenarioControl.value, scenarios);
  const platform = finiteValue(platformControl.value, DESKTOP_PLATFORMS);
  document.documentElement.dataset.view = selected;
  document.documentElement.dataset.platform = platform;
  const view =
    selected === "about"
      ? new LensAboutView()
      : selected === "settings"
        ? new LensSettingsView()
        : selected === "target-selection"
          ? new LensTargetSelectionView()
          : selected === "settings-recovery"
            ? new LensSettingsRecoveryView()
            : new LensOverlayView();
  view.dataset.platform = platform;
  if (view instanceof LensAboutView || view instanceof LensOverlayView)
    view.iconUrl = new URL("./preview-icon.svg", import.meta.url).href;
  if (view instanceof LensSettingsRecoveryView) {
    view.addEventListener("settings-recovery-action", (event) => {
      view.confirming = (event as CustomEvent).detail === "confirm";
    });
    view.info =
      scenario === "loading"
        ? undefined
        : {
            message: "Fixture invalid settings",
            settings_path: "/preview/config.json",
            can_restore_prompt_presets: true,
            digest: "fixture-digest",
          };
    view.error = scenario === "failed" ? "Fixture recovery failure" : "";
  }
  if (view instanceof LensAboutView) {
    view.info =
      scenario === "ready"
        ? {
            stage: "ready",
            value: {
              name: "Lens",
              version,
              copyright: "Lens browser preview",
            },
          }
        : scenario === "failed"
          ? { stage: "failed", message: "Fixture metadata failure" }
          : { stage: "loading" };
    view.documents = {
      stage: "ready",
      value: {
        license: "Preview license document fixture\n\nPreview data",
        notice: "Preview notice document fixture",
      },
    };
    view.releaseAvailability = { revision: 1, stage: "current" };
  }
  if (view instanceof LensSettingsView) {
    view.active = true;
    view.snapshotStatus =
      scenario === "failed"
        ? { stage: "failed", message: "Fixture snapshot failure" }
        : { stage: scenario === "loading" ? "loading" : "ready" };
    view.model =
      scenario === "loading"
        ? undefined
        : {
            platform,
            config: {
              agent: "codex",
              working_directory: "/preview",
              agent_prompt_template: template,
              prompt_presets: {
                schema_version: 2,
                execution_revision: 1,
                revision: 1,
                selected_id: "default",
                presets: [{ id: "default", name: "Preview preset", revision: 1, template }],
              },
            },
            agentSelection: { stage: "unselected", supports_logout: false, auth_methods: [] },
            agentRuntime: { stage: "not_installed", downloaded_bytes: 0 },
            updatePending: false,
            permission: { stage: "required" },
            pending: scenario === "pending",
            promptSynchronization: "preserve-local-draft",
            lensStageLabel: "Idle",
            feedback: { stage: "none" },
          };
  }
  if (view instanceof LensTargetSelectionView) {
    view.snapshotStatus =
      scenario === "failed"
        ? { stage: "failed", message: "Fixture target failure" }
        : { stage: scenario === "loading" ? "loading" : "ready" };
    view.model =
      scenario === "loading"
        ? undefined
        : { platform, lens: targetLens, pending: scenario === "pending", message: "" };
  }
  if (view instanceof LensOverlayView) {
    view.active = true;
    view.snapshotStatus =
      scenario === "failed"
        ? { stage: "failed", message: "Fixture response failure" }
        : { stage: scenario === "loading" ? "loading" : "ready" };
    view.model =
      scenario === "loading"
        ? undefined
        : { platform, lens, pending: scenario === "pending", cancelPending: false, message: "" };
    const answer = lens.output_blocks[0]!;
    if (answer.type !== "markdown") throw new Error("Expected Markdown preview answer");
    view.loadSessionBlock = previewDocumentLoader(scenario, answer);
    view.sessionView = {
      revision: 1,
      phase: "live",
      session_id: "preview",
      conversation: {
        entries: [
          {
            id: "question",
            kind: "message",
            role: "user",
            blocks: [
              { type: "markdown", text: "Can actual components render in a separate consumer?" },
            ],
          },
          {
            id: "answer",
            kind: "message",
            role: "assistant",
            blocks: [
              {
                type: "deferred",
                entry_id: "answer",
                block_index: 0,
                content_type: "markdown",
                revision: 1,
                byte_length: 256,
              },
            ],
          },
        ],
      },
    };
  }
  for (const name of [
    "settings-recovery-action",
    "lens-about-intent",
    "lens-settings-intent",
    "lens-target-selection-intent",
    "lens-overlay-intent",
  ])
    view.addEventListener(name, (event) => {
      intents.push({ event: name, detail: (event as CustomEvent).detail });
      if (intents.length > 32) intents.shift();
      document.querySelector("#intents")!.textContent = JSON.stringify(intents, null, 2);
    });
  mount.replaceChildren(view);
  return view;
}
for (const control of [viewControl, scenarioControl, platformControl])
  control.addEventListener("change", show);
show();
