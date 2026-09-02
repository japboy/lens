import { describe, expect, it } from "vitest";
import {
  MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS,
  agentPromptTemplatesEqual,
  cloneAgentPromptTemplate,
  renderAgentPromptTemplate,
  templatePlaceholderOccurrences,
  validateAgentPromptTemplate,
} from "./agent-prompt-template";
import type { AgentPromptTemplate } from "./types";

function template(): AgentPromptTemplate {
  return {
    schema_version: 1,
    common: "Shared instruction.\n\n{turn_instruction}\n\nObservation boundary.",
    full_projection: "Initial projection.",
    source_checkpoint: "Replace {base_revision} with {target_revision}.",
    current_projection_retry: "Retry {applied_revision}.",
  };
}

describe("Agent prompt template", () => {
  it("renders every finite request mode from visible sections", () => {
    const value = template();

    expect(renderAgentPromptTemplate(value, "full_projection")).toContain("Initial projection.");
    expect(renderAgentPromptTemplate(value, "source_checkpoint")).toContain("Replace 41 with 42.");
    expect(renderAgentPromptTemplate(value, "current_projection_retry")).toContain("Retry 42.");

    value.full_projection = "Preserve {{audience}} literally.";
    expect(renderAgentPromptTemplate(value, "full_projection")).toContain(
      "Preserve {audience} literally.",
    );
  });

  it("validates the same bounded and exhaustive placeholder contract as the backend", () => {
    expect(validateAgentPromptTemplate(template())).toEqual({});

    const missing = { ...template(), common: "Missing the turn variable." };
    expect(validateAgentPromptTemplate(missing).common).toContain("exactly once");

    const unknown = { ...template(), full_projection: "Use {unknown_value}." };
    expect(validateAgentPromptTemplate(unknown).full_projection).toContain("unknown variable");

    const escaped = { ...template(), full_projection: "Use {{unknown_value}} literally." };
    expect(validateAgentPromptTemplate(escaped)).toEqual({});

    const oversized = {
      ...template(),
      full_projection: "x".repeat(MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS + 1),
    };
    expect(validateAgentPromptTemplate(oversized).full_projection).toContain("must not exceed");
  });

  it("reports only active placeholder ranges using textarea-compatible offsets", () => {
    const value = "😀 {{turn_instruction}} then {turn_instruction}.";
    const occurrences = templatePlaceholderOccurrences(value);

    expect(occurrences).toHaveLength(1);
    expect(occurrences[0]?.name).toBe("turn_instruction");
    expect(value.slice(occurrences[0]?.start, occurrences[0]?.end)).toBe("{turn_instruction}");
  });

  it("clones drafts and compares every persisted field", () => {
    const original = template();
    const draft = cloneAgentPromptTemplate(original);

    expect(draft).not.toBe(original);
    expect(agentPromptTemplatesEqual(draft, original)).toBe(true);
    draft.source_checkpoint = "Changed {base_revision} to {target_revision}.";
    expect(agentPromptTemplatesEqual(draft, original)).toBe(false);
  });
});
