#!/usr/bin/env node
//MISE description = "Check public source references"
//MISE dir = "{{config_root}}"

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BINARY_EXTENSIONS,
  repositoryFiles,
  UTF8_DECODER,
} from "../../scripts/repository-files.ts";

const REPOSITORY_ROOT = resolve(fileURLToPath(new URL("../../", import.meta.url)));
const POLICY_FILE = "mise-tasks/check/publication.ts";
const DEFINITION_FILE = ".gitignore";
const IGNORED_RESOURCE_REFERENCES = [
  "ARCHITECTURE.md",
  "AGENTS.md",
  ["BASIC", "_DESIGN.md"].join(""),
  [".agents", "/"].join(""),
  [".serena", "/"].join(""),
  ["docs", "/"].join(""),
];

export function ignoredResourceReferences(line: string): string[] {
  // Absolute web URLs refer to external resources, not this checkout.
  const localText = line.replace(/https?:\/\/[^\s<>"'`()[\]{}]+/giu, "");
  return IGNORED_RESOURCE_REFERENCES.filter((reference) => localText.includes(reference));
}

function scanFile(relativePath: string): string[] {
  if (relativePath === DEFINITION_FILE || relativePath === POLICY_FILE) return [];
  if (BINARY_EXTENSIONS.has(extname(relativePath).toLowerCase())) return [];

  const content = readFileSync(resolve(REPOSITORY_ROOT, relativePath));
  let text: string;
  try {
    text = UTF8_DECODER.decode(content);
  } catch {
    return [];
  }

  const violations = [];
  for (const [index, line] of text.split(/\r?\n/u).entries()) {
    for (const reference of ignoredResourceReferences(line)) {
      violations.push(
        `${relativePath}:${index + 1}: references ignored local resource ${JSON.stringify(reference)}`,
      );
    }
  }
  return violations;
}

function run(): void {
  const files = repositoryFiles(REPOSITORY_ROOT);
  const violations = files.flatMap(scanFile);

  if (violations.length > 0) {
    process.stderr.write(["Publication boundary policy failed:", ...violations].join("\n") + "\n");
    process.exitCode = 1;
  } else {
    process.stdout.write(
      `Publication boundary policy passed: no disallowed ignored-resource references across ${files.length} source artifacts.\n`,
    );
  }
}

const entryPoint = process.argv[1];
if (entryPoint && fileURLToPath(import.meta.url) === resolve(entryPoint)) run();
