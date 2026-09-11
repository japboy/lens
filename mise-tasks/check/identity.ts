#!/usr/bin/env node
//MISE description = "Check product identity"
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
const LEGACY_PRODUCT_IDENTITY = /personal(?:[-_ ]?lens)/iu;
const LEGACY_PRODUCT_ABBREVIATION = /\b(?:PL_[A-Z0-9_]+|pl_[a-z0-9_]+)\b/u;

function scanFile(relativePath: string): string[] {
  const violations = [];
  if (
    LEGACY_PRODUCT_IDENTITY.test(relativePath) ||
    LEGACY_PRODUCT_ABBREVIATION.test(relativePath)
  ) {
    violations.push(`${relativePath}: file path uses a legacy product identity`);
  }
  // Historical commit subjects retain the product names used at the time.
  if (relativePath === "CHANGELOG.md" || BINARY_EXTENSIONS.has(extname(relativePath).toLowerCase()))
    return violations;

  const content = readFileSync(resolve(REPOSITORY_ROOT, relativePath));
  let text: string;
  try {
    text = UTF8_DECODER.decode(content);
  } catch {
    return violations;
  }

  for (const [index, line] of text.split(/\r?\n/u).entries()) {
    if (LEGACY_PRODUCT_IDENTITY.test(line) || LEGACY_PRODUCT_ABBREVIATION.test(line)) {
      violations.push(`${relativePath}:${index + 1}: uses a legacy product identity`);
    }
  }
  return violations;
}

const files = repositoryFiles(REPOSITORY_ROOT);
const violations = files.flatMap(scanFile);

if (violations.length > 0) {
  process.stderr.write(["Product identity policy failed:", ...violations].join("\n") + "\n");
  process.exitCode = 1;
} else {
  process.stdout.write(
    `Product identity policy passed: no legacy aliases across ${files.length} source artifacts.\n`,
  );
}
