#!/usr/bin/env node
//MISE description = "Check product terminology"
//MISE dir = "{{config_root}}"

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPOSITORY_ROOT = resolve(fileURLToPath(new URL("../../", import.meta.url)));
const PRODUCT_PATHS = new Set([
  "README.md",
  "apps/desktop/src-tauri/tauri.conf.json",
  "apps/desktop/src-tauri/tauri.macos.conf.json",
]);
const PRODUCT_PREFIXES = [
  "apps/desktop/src/",
  "apps/desktop/src-tauri/src/",
  "apps/desktop/src-tauri/native/",
  "packages/",
  "apps/desktop/public/",
] as const;
const BINARY_EXTENSIONS = new Set([".icns", ".ico", ".png"]);
const LEGACY_OUTPUT_TERM = /translation/iu;
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

// Exact, reviewed uses of the language-conversion term, never whole-file exemptions.
const ALLOWED_USAGES = [
  {
    path: "README.md",
    line: "- `Translation` is reserved for actual conversion between human languages.",
    reason: "Defines the narrower language-conversion meaning in the public terminology contract.",
  },
] as const;

export function isProductTerminologyPath(path: string): boolean {
  return PRODUCT_PATHS.has(path) || PRODUCT_PREFIXES.some((prefix) => path.startsWith(prefix));
}

export function productTerminologyViolations(path: string, content: string): string[] {
  if (!isProductTerminologyPath(path)) return [];

  const violations: string[] = [];
  if (LEGACY_OUTPUT_TERM.test(path)) {
    violations.push(`${path}: product source path must use Interpretation`);
  }
  for (const [index, line] of content.split(/\r?\n/u).entries()) {
    if (!LEGACY_OUTPUT_TERM.test(line)) continue;
    if (ALLOWED_USAGES.some((usage) => usage.path === path && usage.line === line)) continue;
    violations.push(
      `${path}:${index + 1}: use Interpretation for Lens output; language-conversion terms require an exact documented policy allowance`,
    );
  }
  return violations;
}

function run(): void {
  const output = execFileSync(
    "git",
    ["-C", REPOSITORY_ROOT, "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { encoding: "utf8" },
  );
  const files = [...new Set(output.split("\0"))]
    .filter((path) => isProductTerminologyPath(path) && existsSync(resolve(REPOSITORY_ROOT, path)))
    .sort();
  const violations = files.flatMap((path) => {
    let content = "";
    if (!BINARY_EXTENSIONS.has(extname(path).toLowerCase())) {
      const bytes = readFileSync(resolve(REPOSITORY_ROOT, path));
      try {
        content = UTF8_DECODER.decode(bytes);
      } catch {
        // The repository-language policy owns invalid UTF-8 diagnostics.
      }
    }
    return productTerminologyViolations(path, content);
  });

  if (violations.length > 0) {
    process.stderr.write(["Product terminology policy failed:", ...violations].join("\n") + "\n");
    process.exitCode = 1;
  } else {
    process.stdout.write(`Product terminology policy passed: ${files.length} product artifacts.\n`);
  }
}

const entryPoint = process.argv[1];
if (entryPoint && fileURLToPath(import.meta.url) === resolve(entryPoint)) run();
