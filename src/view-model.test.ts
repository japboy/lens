import { describe, expect, it } from "vitest";
import type { AgentRuntimeStage, AgentSelectionStage, LensStage, LensState } from "./types";
import {
  AGENT_RUNTIME_LABEL,
  AGENT_SELECTION_LABEL,
  imageDataUrl,
  isAgentRuntimeActive,
  lensOutputBlocks,
  lensSourceJson,
  selectedAgent,
  showsLensProgress,
  shouldApplySnapshot,
  STAGE_LABEL,
  supportedAuthMethods,
} from "./view-model";

describe("PersonalLens view model", () => {
  it("defines every managed Agent runtime stage and its active states", () => {
    const stages: AgentRuntimeStage[] = [
      "not_installed",
      "resolving",
      "downloading",
      "verifying",
      "installing",
      "ready",
      "failed",
    ];

    expect(Object.keys(AGENT_RUNTIME_LABEL)).toEqual(stages);
    expect(stages.filter(isAgentRuntimeActive)).toEqual([
      "resolving",
      "downloading",
      "verifying",
      "installing",
    ]);
  });

  it("defines every Agent selection stage and enables Lens selection only when selected", () => {
    const stages: AgentSelectionStage[] = [
      "unselected",
      "checking",
      "authentication_required",
      "authenticating",
      "signing_out",
      "selected",
      "failed",
    ];

    expect(Object.keys(AGENT_SELECTION_LABEL)).toEqual(stages);
    for (const stage of stages) {
      const selection = { stage, candidate: "codex" as const, auth_methods: [] };
      expect(selectedAgent(selection)).toBe(stage === "selected" ? "codex" : undefined);
    }
  });

  it("defines a user-facing label for every finite Lens stage", () => {
    const stages: LensStage[] = [
      "idle",
      "selecting",
      "extracting",
      "ready",
      "connecting",
      "authentication_required",
      "transforming",
      "completed",
      "cancelled",
      "failed",
    ];

    expect(Object.keys(STAGE_LABEL).sort()).toEqual([...stages].sort());
    expect(stages.every((stage) => STAGE_LABEL[stage].length > 0)).toBe(true);
  });

  it("preserves ordered typed output without falling back to extraction text", () => {
    const lens = {
      stage: "transforming",
      output_blocks: [
        { type: "markdown", message_id: "message-1", text: "Agent output" },
        {
          type: "image",
          message_id: "message-1",
          mime_type: "image/png",
          data: "iVBORw0KGgo=",
        },
      ],
      input: {
        source: {
          application: "Safari",
          window_title: "Fixture",
          bundle_id: "com.apple.Safari",
          window_id: 417,
        },
        text: "Accessibility input",
        extraction_quality: "full",
      },
    } satisfies LensState;

    expect(lensOutputBlocks(lens)).toEqual(lens.output_blocks);
    const imageBlock = lens.output_blocks[1];
    expect(imageBlock?.type).toBe("image");
    if (imageBlock?.type !== "image") throw new Error("expected image output block");
    expect(imageDataUrl(imageBlock)).toBe("data:image/png;base64,iVBORw0KGgo=");
    expect(lensOutputBlocks({ ...lens, output_blocks: [] })).toEqual([]);
  });

  it("rejects image MIME types outside the renderer allowlist", () => {
    expect(
      imageDataUrl({ type: "image", mime_type: "image/svg+xml", data: "PHN2Zz4=" }),
    ).toBeUndefined();
  });

  it("pretty-prints only the normalized LensInput in the Source view", () => {
    const lens = {
      stage: "ready",
      output_blocks: [],
      extraction: {
        quality: "full",
        text: "Raw extraction must not become a second Source authority.",
        diagnostics: [],
        metrics: {
          visited_nodes: 1,
          text_bytes: 10,
          offscreen_text_nodes: 0,
          virtualization_signals: 0,
          truncated_nodes: false,
          truncated_text: false,
          children_read_errors: 0,
        },
      },
      input: {
        source: {
          application: "Safari",
          window_title: "Fixture",
          bundle_id: "com.apple.Safari",
          window_id: 417,
        },
        text: "Normalized source",
        extraction_quality: "full",
      },
    } satisfies LensState;

    const sourceJson = lensSourceJson(lens);
    expect(sourceJson).toBe(JSON.stringify(lens.input, undefined, 2));
    expect(JSON.parse(sourceJson)).toEqual(lens.input);
    expect(lensSourceJson({ ...lens, input: undefined })).toBe("");
  });

  it("offers only authentication methods the client supports", () => {
    const lens = {
      stage: "authentication_required",
      output_blocks: [],
      agent: {
        run_id: "0198e6de-d046-7bf2-b8b2-d84cfaba7e2d",
        kind: "claude",
        adapter_name: "@agentclientprotocol/claude-agent-acp",
        adapter_version: "0.70.0",
        auth_methods: [
          { id: "terminal", name: "Terminal", kind: "terminal", supported: true },
          {
            id: "environment",
            name: "Environment",
            kind: "environment_variable",
            supported: false,
          },
        ],
        received_updates: 0,
      },
    } satisfies LensState;

    expect(supportedAuthMethods(lens).map((method) => method.id)).toEqual(["terminal"]);
  });

  it("shows progress only while an operation can still advance without user input", () => {
    expect(
      ["selecting", "extracting", "ready", "connecting", "transforming"].every((stage) =>
        showsLensProgress(stage as LensStage),
      ),
    ).toBe(true);
    expect(
      ["idle", "authentication_required", "completed", "cancelled", "failed"].every(
        (stage) => !showsLensProgress(stage as LensStage),
      ),
    ).toBe(true);
  });

  it("accepts only a strictly newer finite application snapshot", () => {
    expect(shouldApplySnapshot(-1, 0)).toBe(true);
    expect(shouldApplySnapshot(7, 8)).toBe(true);
    expect(shouldApplySnapshot(7, 7)).toBe(false);
    expect(shouldApplySnapshot(7, 6)).toBe(false);
    expect(shouldApplySnapshot(7, Number.NaN)).toBe(false);
    expect(shouldApplySnapshot(7, 7.5)).toBe(false);
  });
});
