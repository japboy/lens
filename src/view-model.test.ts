import { describe, expect, it } from "vitest";
import type { AgentRuntimeStage, AgentSelectionStage, LensStage, LensState } from "./types";
import {
  AGENT_RUNTIME_LABEL,
  AGENT_SELECTION_LABEL,
  imageDataUrl,
  inputMediaPreviewUrl,
  isAgentRuntimeActive,
  lensProgressSnackbar,
  lensOutputBlocks,
  lensSourceJson,
  selectedAgent,
  shouldApplySnapshot,
  STAGE_LABEL,
  supportedAuthMethods,
} from "./view-model";

function lensInput(text: string): NonNullable<LensState["input"]> {
  return {
    schema_version: 3,
    context_id: "0198e6de-d046-7bf2-b8b2-d84cfaba7e2d",
    context_revision: 1,
    sources: [
      {
        source_id: "macos:com.apple.Safari:417:accessibility",
        target_id: "macos:com.apple.Safari:417",
        source_revision: 1,
        source: {
          application: "Safari",
          window_title: "Fixture",
          bundle_id: "com.apple.Safari",
          window_id: 417,
        },
        document: {
          nodes: [
            {
              id: "node-000000",
              kind: "text",
              value: text,
            },
          ],
        },
        quality: "full",
        omissions: [],
      },
    ],
    media: [],
    media_omissions: [],
    quality: "full",
  };
}

describe("Lens view model", () => {
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
      input: lensInput("Accessibility input"),
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
      context: {
        schema_version: 4,
        context_id: "0198e6de-d046-7bf2-b8b2-d84cfaba7e2d",
        revision: 1,
        sources: [
          {
            source_id: "macos:com.apple.Safari:417:accessibility",
            target_id: "macos:com.apple.Safari:417",
            revision: 1,
            source: {
              application: "Safari",
              window_title: "Fixture",
              bundle_id: "com.apple.Safari",
              window_id: 417,
            },
            capture: {
              quality: "full",
              nodes: [],
              text: "",
              diagnostics: [],
              metrics: {
                visited_nodes: 0,
                text_bytes: 0,
                offscreen_text_nodes: 0,
                virtualization_signals: 0,
                truncated_nodes: false,
                truncated_text: false,
                children_read_errors: 0,
                resource_ref_count: 0,
                resource_uri_bytes: 0,
                omitted_resource_refs: 0,
                resource_read_errors: 0,
              },
            },
            quality: "full",
          },
        ],
        media: [],
        media_omissions: [],
        quality: "full",
        diagnostics: ["Raw capture diagnostics must not become a second Source authority."],
      },
      input: lensInput("Normalized source"),
    } satisfies LensState;

    const sourceJson = lensSourceJson(lens);
    expect(sourceJson).toBe(JSON.stringify(lens.input, undefined, 2));
    expect(JSON.parse(sourceJson)).toEqual(lens.input);
    expect(lensSourceJson({ ...lens, input: undefined })).toBe("");
  });

  it("accepts only current operation-scoped PNG input preview URIs", () => {
    const input = lensInput("Normalized source");
    const attachment = {
      id: "media-node-000001",
      target_id: "macos:com.apple.Safari:417",
      uri: `lens://context/${input.context_id}/${input.context_revision}/media/media-node-000001`,
      scope: "ax_element_region",
      source_node_id: "node-000001",
      source_bounds: { x: 10, y: 20, width: 30, height: 40 },
      captured_bounds: { x: 10, y: 20, width: 30, height: 40 },
      coverage: "full_region",
      coordinate_space: "screen_points",
      mime_type: "image/png",
      pixel_width: 300,
      pixel_height: 400,
      encoded_bytes: 512,
    } as const;
    input.media = [attachment];
    const lens = {
      operation_id: input.context_id,
      stage: "ready",
      output_blocks: [],
      input,
    } satisfies LensState;

    expect(inputMediaPreviewUrl(lens, attachment)).toBe(attachment.uri);
    expect(
      inputMediaPreviewUrl({ ...lens, operation_id: "superseding-operation" }, attachment),
    ).toBeUndefined();
    expect(
      inputMediaPreviewUrl(lens, { ...attachment, uri: "https://example.com/image.png" }),
    ).toBeUndefined();
    expect(
      inputMediaPreviewUrl(lens, { ...attachment, mime_type: "image/svg+xml" }),
    ).toBeUndefined();
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

  it("describes a snackbar only while Lens is changing state", () => {
    const changingStages: LensStage[] = ["selecting", "extracting", "connecting", "transforming"];
    const stableStages: LensStage[] = [
      "idle",
      "ready",
      "authentication_required",
      "completed",
      "cancelled",
      "failed",
    ];

    for (const stage of changingStages) {
      expect(lensProgressSnackbar(stage)).toMatchObject({ title: STAGE_LABEL[stage] });
      expect(lensProgressSnackbar(stage)?.detail.length).toBeGreaterThan(0);
    }
    for (const stage of stableStages) {
      expect(lensProgressSnackbar(stage)).toBeUndefined();
    }
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
