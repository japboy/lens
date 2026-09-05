import { appendFileSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const CI_IMPACT_STATES = ["portable-only", "native-or-control-plane"] as const;

export type CiImpactState = (typeof CI_IMPACT_STATES)[number];

const CONTROL_PLANE_PATHS = new Set([
  "scripts/classify-ci-impact.test.ts",
  "scripts/classify-ci-impact.ts",
  "scripts/workspace-tasks.test.ts",
  "scripts/workspace-policy.ts",
  "scripts/check-workspace-boundaries.ts",
  "scripts/rust-source-boundaries.ts",
  "scripts/workspace-boundaries.test.ts",
  "scripts/workspace-feature-graphs.ts",
  "scripts/workspace-feature-graphs.test.ts",
  "scripts/rust-source-surface.ts",
  "scripts/rust-source-surface.test.ts",
  "scripts/run-workspace-variant.ts",
  "scripts/run-workspace-variant.test.ts",
  "scripts/workspace-variants.json",
  "scripts/check-portable-native-features.ts",
  "scripts/check-portable-native-features.test.ts",
]);

const PORTABLE_PATHS = new Set([
  ".editorconfig",
  ".gitignore",
  "LICENSE",
  "README.md",
  "index.html",
  "oxfmt.config.ts",
  "oxlint.config.ts",
  "tsconfig.app.json",
  "tsconfig.json",
  "vite.config.ts",
]);

const PORTABLE_PREFIXES = ["public/", "scripts/", "src/", "tests/fixtures/"] as const;

function isPortablePath(path: string): boolean {
  if (CONTROL_PLANE_PATHS.has(path)) return false;
  return PORTABLE_PATHS.has(path) || PORTABLE_PREFIXES.some((prefix) => path.startsWith(prefix));
}

export function classifyCiImpact(changedPaths: readonly string[]): CiImpactState {
  return changedPaths.length > 0 && changedPaths.every(isPortablePath)
    ? "portable-only"
    : "native-or-control-plane";
}

function run(): void {
  const changedPaths = readFileSync(0, "utf8").split("\0").filter(Boolean).sort();
  const impact = classifyCiImpact(changedPaths);
  const githubOutput = process.env.GITHUB_OUTPUT;

  if (!githubOutput) throw new Error("GITHUB_OUTPUT is required");

  appendFileSync(githubOutput, `impact=${impact}\n`);
  process.stdout.write(
    `CI impact: ${impact} (${changedPaths.length} changed path${changedPaths.length === 1 ? "" : "s"})\n`,
  );
}

const entryPoint = process.argv[1];
if (entryPoint && fileURLToPath(import.meta.url) === resolve(entryPoint)) run();
