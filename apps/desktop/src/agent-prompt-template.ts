import type { AgentPromptTemplate } from "./types";

export const AGENT_PROMPT_TEMPLATE_SCHEMA_VERSION = 1;
export const MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS = 16_384;

export type PromptTemplateSection =
  | "common"
  | "full_projection"
  | "source_checkpoint"
  | "current_projection_retry";

export type AgentPromptPreviewMode =
  | "full_projection"
  | "source_checkpoint"
  | "current_projection_retry";

export type SettingsDestination = "general" | "agent-prompt";

export type PromptEditorLayer = "shared" | "request";

export type PromptPlaceholderName =
  | "turn_instruction"
  | "base_revision"
  | "target_revision"
  | "applied_revision";

export interface PromptVariableDescriptor {
  name: PromptPlaceholderName;
  token: string;
  label: string;
  description: string;
}

export interface PromptSectionDescriptor {
  key: PromptTemplateSection;
  layer: PromptEditorLayer;
  title: string;
  description: string;
  variables: readonly PromptVariableDescriptor[];
}

export interface PromptRequestSectionDescriptor extends PromptSectionDescriptor {
  key: AgentPromptPreviewMode;
  layer: "request";
}

function promptVariable(
  name: PromptPlaceholderName,
  label: string,
  description: string,
): PromptVariableDescriptor {
  return { name, token: `{${name}}`, label, description };
}

export const SHARED_PROMPT_SECTION: PromptSectionDescriptor = {
  key: "common",
  layer: "shared",
  title: "Shared Instructions",
  description:
    "Applies to every Agent request and places the selected request instruction at its tag.",
  variables: [
    promptVariable(
      "turn_instruction",
      "Request instruction",
      "Inserts the instruction for the selected request type.",
    ),
  ],
};

export const REQUEST_PROMPT_SECTIONS: readonly PromptRequestSectionDescriptor[] = [
  {
    key: "full_projection",
    layer: "request",
    title: "Initial Request",
    description: "Defines the turn-specific instruction for the first canonical projection.",
    variables: [],
  },
  {
    key: "source_checkpoint",
    layer: "request",
    title: "Source Update",
    description:
      "Defines the instruction used when a newer full projection replaces the session source.",
    variables: [
      promptVariable("base_revision", "Previous revision", "Inserts the revision being replaced."),
      promptVariable("target_revision", "New revision", "Inserts the authoritative new revision."),
    ],
  },
  {
    key: "current_projection_retry",
    layer: "request",
    title: "Retry Current Projection",
    description:
      "Defines the instruction used to regenerate output for the projection already applied.",
    variables: [
      promptVariable(
        "applied_revision",
        "Current revision",
        "Inserts the revision that remains authoritative for the retry.",
      ),
    ],
  },
];

export const PROMPT_SECTIONS: readonly PromptSectionDescriptor[] = [
  SHARED_PROMPT_SECTION,
  ...REQUEST_PROMPT_SECTIONS,
];

const PLACEHOLDER_NAME = /^[a-z][a-z_]*$/;

export function cloneAgentPromptTemplate(template: AgentPromptTemplate): AgentPromptTemplate {
  return { ...template };
}

export function agentPromptTemplatesEqual(
  first: AgentPromptTemplate | undefined,
  second: AgentPromptTemplate | undefined,
): boolean {
  return (
    first?.schema_version === second?.schema_version &&
    first?.common === second?.common &&
    first?.full_projection === second?.full_projection &&
    first?.source_checkpoint === second?.source_checkpoint &&
    first?.current_projection_retry === second?.current_projection_retry
  );
}

export function promptSectionDescriptor(section: PromptTemplateSection): PromptSectionDescriptor {
  const descriptor = PROMPT_SECTIONS.find((candidate) => candidate.key === section);
  if (!descriptor) throw new Error(`Unknown prompt template section: ${section}`);
  return descriptor;
}

export function promptRequestSectionDescriptor(
  mode: AgentPromptPreviewMode,
): PromptRequestSectionDescriptor {
  const descriptor = REQUEST_PROMPT_SECTIONS.find((candidate) => candidate.key === mode);
  if (!descriptor) throw new Error(`Unknown Agent prompt request mode: ${mode}`);
  return descriptor;
}

export function validateAgentPromptTemplate(
  template: AgentPromptTemplate,
): Partial<Record<PromptTemplateSection | "schema_version", string>> {
  const errors: Partial<Record<PromptTemplateSection | "schema_version", string>> = {};
  if (template.schema_version !== AGENT_PROMPT_TEMPLATE_SCHEMA_VERSION) {
    errors.schema_version = `Schema version must be ${AGENT_PROMPT_TEMPLATE_SCHEMA_VERSION}.`;
  }
  for (const descriptor of PROMPT_SECTIONS) {
    const error = validateSection(descriptor, template[descriptor.key]);
    if (error) errors[descriptor.key] = error;
  }
  return errors;
}

export function renderAgentPromptTemplate(
  template: AgentPromptTemplate,
  mode: AgentPromptPreviewMode,
): string {
  const turnInstruction = (() => {
    switch (mode) {
      case "full_projection":
        return renderSection(template.full_projection, {});
      case "source_checkpoint":
        return renderSection(template.source_checkpoint, {
          base_revision: "41",
          target_revision: "42",
        });
      case "current_projection_retry":
        return renderSection(template.current_projection_retry, { applied_revision: "42" });
    }
  })();
  return renderSection(template.common, { turn_instruction: turnInstruction });
}

function validateSection(descriptor: PromptSectionDescriptor, value: string): string | undefined {
  if (!value.trim()) return `${descriptor.title} must not be empty.`;
  const length = [...value].length;
  if (length > MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS) {
    return `${descriptor.title} must not exceed ${MAX_AGENT_PROMPT_TEMPLATE_SECTION_CHARS.toLocaleString()} characters.`;
  }

  const allowed: readonly string[] = descriptor.variables.map(({ name }) => name);
  const counts = templatePlaceholders(value);
  const unknown = [...counts.keys()].find((name) => !allowed.includes(name));
  if (unknown) return `${descriptor.title} contains unknown variable {${unknown}}.`;
  const invalidRequired = allowed.find((name) => counts.get(name) !== 1);
  if (invalidRequired) {
    return `${descriptor.title} must contain {${invalidRequired}} exactly once.`;
  }
  return undefined;
}

export interface TemplatePlaceholderOccurrence {
  name: string;
  start: number;
  end: number;
}

export function templatePlaceholderOccurrences(
  value: string,
): readonly TemplatePlaceholderOccurrence[] {
  const occurrences: TemplatePlaceholderOccurrence[] = [];
  for (let index = 0; index < value.length;) {
    const pair = value.slice(index, index + 2);
    if (pair === "{{" || pair === "}}") {
      index += 2;
      continue;
    }
    if (value[index] === "{") {
      const end = value.indexOf("}", index + 1);
      if (end >= 0) {
        const name = value.slice(index + 1, end);
        if (PLACEHOLDER_NAME.test(name)) {
          occurrences.push({ name, start: index, end: end + 1 });
          index = end + 1;
          continue;
        }
      }
    }
    const codePoint = value.codePointAt(index);
    index += codePoint !== undefined && codePoint > 0xffff ? 2 : 1;
  }
  return occurrences;
}

function templatePlaceholders(value: string): Map<string, number> {
  const placeholders = new Map<string, number>();
  for (const { name } of templatePlaceholderOccurrences(value)) {
    placeholders.set(name, (placeholders.get(name) ?? 0) + 1);
  }
  return placeholders;
}

function renderSection(template: string, values: Readonly<Record<string, string>>): string {
  let rendered = "";
  for (let index = 0; index < template.length;) {
    const pair = template.slice(index, index + 2);
    if (pair === "{{") {
      rendered += "{";
      index += 2;
      continue;
    }
    if (pair === "}}") {
      rendered += "}";
      index += 2;
      continue;
    }
    if (template[index] === "{") {
      const end = template.indexOf("}", index + 1);
      if (end >= 0) {
        const candidate = template.slice(index + 1, end);
        if (PLACEHOLDER_NAME.test(candidate)) {
          rendered += values[candidate] ?? template.slice(index, end + 1);
          index = end + 1;
          continue;
        }
      }
    }
    const codePoint = template.codePointAt(index);
    if (codePoint === undefined) break;
    rendered += String.fromCodePoint(codePoint);
    index += codePoint > 0xffff ? 2 : 1;
  }
  return rendered;
}
