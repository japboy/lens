import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPOSITORY_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const POLICY_FILE = "scripts/check-publication-boundary.ts";
const DEFINITION_FILE = ".gitignore";
const BINARY_EXTENSIONS = new Set([".icns", ".ico", ".png"]);
const IGNORED_RESOURCE_REFERENCES = [
  "ARCHITECTURE.md",
  "AGENTS.md",
  ["BASIC", "_DESIGN.md"].join(""),
  [".agents", "/"].join(""),
  [".serena", "/"].join(""),
  ["docs", "/"].join(""),
];
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

function repositoryFiles(): string[] {
  const output = execFileSync(
    "git",
    ["-C", REPOSITORY_ROOT, "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { encoding: "utf8" },
  );

  return output
    .split("\0")
    .filter(
      (relativePath) => Boolean(relativePath) && existsSync(resolve(REPOSITORY_ROOT, relativePath)),
    )
    .sort();
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
    for (const reference of IGNORED_RESOURCE_REFERENCES) {
      if (line.includes(reference)) {
        violations.push(
          `${relativePath}:${index + 1}: references ignored local resource ${JSON.stringify(reference)}`,
        );
      }
    }
  }
  return violations;
}

const files = repositoryFiles();
const violations = files.flatMap(scanFile);

if (violations.length > 0) {
  process.stderr.write(["Publication boundary policy failed:", ...violations].join("\n") + "\n");
  process.exitCode = 1;
} else {
  process.stdout.write(
    `Publication boundary policy passed: no disallowed ignored-resource references across ${files.length} source artifacts.\n`,
  );
}
