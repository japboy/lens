import { describe, expect, it } from "vitest";
import type { SettingsIntent } from "../components/events";
import type { AccessibilityPermissionState } from "./accessibility-permission-controller";
import type { SnapshotConnectionState } from "./app-snapshot-controller";
import type { CommandState } from "./command-state";
import { settingsViewModel } from "./view-models";
import type { AppSnapshot } from "../types";

const SNAPSHOT: AppSnapshot = {
  revision: 1,
  config: {
    agent: "codex",
    working_directory: "/tmp",
    agent_prompt_template: {
      schema_version: 1,
      common: "{turn_instruction}",
      full_projection: "Initial",
      source_checkpoint: "{base_revision} {target_revision}",
      current_projection_retry: "{applied_revision}",
    },
    prompt_presets: {
      schema_version: 2,
      execution_revision: 1,
      revision: 1,
      selected_id: "visual-learner",
      presets: [
        {
          id: "visual-learner",
          name: "Visual Learner",

          revision: 1,
          template: {
            schema_version: 1,
            common: "{turn_instruction}",
            full_projection: "Initial",
            source_checkpoint: "{base_revision} {target_revision}",
            current_projection_retry: "{applied_revision}",
          },
        },
      ],
    },
  },
  agent_selection: { stage: "unselected", auth_methods: [] },
  agent_runtime: { stage: "not_installed", downloaded_bytes: 0 },
  lens: { stage: "idle", prompt_execution_revision: 1, output_blocks: [] },
};

const PERMISSION: AccessibilityPermissionState = { stage: "allowed" };
const READY_CONNECTION: SnapshotConnectionState = { stage: "ready" };

function model(
  command: CommandState = { stage: "idle" },
  connection: SnapshotConnectionState = READY_CONNECTION,
) {
  return settingsViewModel("macos", SNAPSHOT, PERMISSION, command, connection);
}

function succeeded(type: Exclude<SettingsIntent["type"], "open-about">): CommandState {
  return {
    stage: "succeeded",
    command: { scope: "settings", type },
    message: `${type} completed`,
  };
}

describe("Settings view model", () => {
  it("keeps the application-global Lens stage independent from contextual feedback", () => {
    expect(model()).toMatchObject({
      lensStageLabel: "Idle",
      feedback: { stage: "none" },
    });
  });

  it("maps every General command result to the General destination", () => {
    const generalCommands = [
      "select-agent",
      "authenticate-agent-selection",
      "reauthenticate-agent-selection",
      "sign-out-agent-selection",
      "choose-directory",
    ] as const satisfies readonly SettingsIntent["type"][];

    for (const type of generalCommands) {
      expect(model(succeeded(type)).feedback).toEqual({
        stage: "status",
        target: "connection",
        message: `${type} completed`,
      });
    }
  });

  it("maps every Agent Prompt command result to the Agent Prompt destination", () => {
    const promptCommands = [
      "save-agent-prompt-template",
      "reset-agent-prompt-template",
    ] as const satisfies readonly SettingsIntent["type"][];

    for (const type of promptCommands) {
      expect(model(succeeded(type)).feedback).toEqual({
        stage: "status",
        target: "prompt-presets",
        message: `${type} completed`,
      });
    }
  });

  it("keeps connection progress advisory and connection failures explicit", () => {
    expect(model({ stage: "idle" }, { stage: "loading" }).feedback).toEqual({
      stage: "status",
      target: "application",
      message: "Loading application state…",
    });
    expect(
      model({ stage: "idle" }, { stage: "failed", message: "Connection failed" }).feedback,
    ).toEqual({
      stage: "error",
      target: "application",
      message: "Connection failed",
    });
  });

  it("does not leak command feedback from another window into Settings", () => {
    expect(
      model({
        stage: "failed",
        command: { scope: "overlay", type: "retry" },
        message: "Overlay retry failed",
      }).feedback,
    ).toEqual({ stage: "none" });
  });
});
