import { portableSourceViolations } from "./rust-source-boundaries.ts";
import { rustDeclarationSurface } from "./rust-source-surface.ts";

// Required work is explicit policy data, not a package-name or file-extension guess.
export const CI_PLANS = {
  "frontend-only": { common: false, native: "none" },
  "portable-rust": { common: true, native: "none" },
  "native-code": { common: true, native: "code" },
  "native-bundle": { common: true, native: "bundle" },
  full: { common: true, native: "bundle" },
} as const;
export type CiPlan = keyof typeof CI_PLANS;
export type Change = { path: string; before: string | null; after: string | null };
export type Result = "success" | "failure" | "cancelled" | "skipped";

const FRONTEND_CONTRACTS = new Set([
  "apps/desktop/src/types.ts",
  "apps/desktop/src/agent-prompt-template.ts",
  "apps/desktop/src/output-media.ts",
  "apps/desktop/src/presentation-context.ts",
  "apps/desktop/src/application/webview-port.ts",
  "apps/desktop/src/application/accessibility-permission-controller.ts",
]);

export function validateChangedPath(path: string): void {
  if (
    !path ||
    path.startsWith("/") ||
    [...path].some(
      (character) =>
        character.codePointAt(0)! < 32 ||
        character === "\u007f" ||
        character === "\ufffd" ||
        character === "\\",
    ) ||
    path.split("/").some((part) => !part || part === "." || part === "..")
  )
    throw new Error("Invalid changed repository path");
}

export function parseChangedPaths(input: string): string[] {
  if (!input) return [];
  if (!input.endsWith("\0")) throw new Error("Incomplete NUL-delimited change list");
  const paths = input.slice(0, -1).split("\0");
  for (const path of paths) validateChangedPath(path);
  if (new Set(paths).size !== paths.length) throw new Error("Duplicate changed path");
  return paths.sort();
}

export function classifyChange(change: Change): CiPlan {
  const { path } = change;
  validateChangedPath(path);
  // Manifests, dependencies, toolchains, policies and unknown extensions are control
  // plane even if placed under an otherwise portable source directory.
  if (
    /(?:^|\/)(?:Cargo\.toml|Cargo\.lock|package\.json|pnpm-lock\.yaml|pnpm-workspace\.yaml)$/u.test(
      path,
    ) ||
    path.startsWith("scripts/") ||
    path.startsWith("mise-tasks/") ||
    path.startsWith("packages/typescript-config/") ||
    path.startsWith(".github/") ||
    path.startsWith(".cargo/") ||
    /^apps\/desktop\/src-tauri\/tauri\.(?:conf|macos\.conf|release\.conf)\.json$/u.test(path)
  )
    return "full";
  if (FRONTEND_CONTRACTS.has(path)) return "native-code";
  if (
    path.startsWith("packages/adapter-platform-macos/") ||
    path.startsWith("apps/desktop/src-tauri/src/native/") ||
    path === "apps/desktop/src-tauri/src/lib.rs" ||
    path === "apps/desktop/src-tauri/src/main.rs"
  )
    return "native-bundle";
  if (
    path.startsWith("packages/port-platform/") ||
    /^apps\/desktop\/src-tauri\/src\/.+\.rs$/u.test(path)
  )
    return "native-code";
  if (path.startsWith("apps/desktop/src-tauri/")) return "native-bundle";
  if (/^packages\/(?:domain|usecase)\/src\/.+\.rs$/u.test(path)) {
    if (change.before === null || change.after === null) return "native-code";
    for (const source of [change.before, change.after]) {
      const violations = portableSourceViolations(source);
      if (violations.length)
        throw new Error(`Portable boundary failure: ${path}: ${violations.join("; ")}`);
    }
    return rustDeclarationSurface(change.before) === rustDeclarationSurface(change.after)
      ? "portable-rust"
      : "native-code";
  }
  if (
    /^apps\/desktop\/src\/.+\.(?:ts|css|svg)$/u.test(path) ||
    /^apps\/desktop\/(?:public|tests\/fixtures)\/.+\.(?:html|css|ts|js|json|svg|png|jpg|webp)$/u.test(
      path,
    ) ||
    /^apps\/desktop\/[^/]+\.html$/u.test(path) ||
    path === "README.md" ||
    path === "LICENSE"
  )
    return "frontend-only";
  return "full";
}

export function planChanges(changes: readonly Change[]): CiPlan {
  if (!changes.length) return "full";
  const paths = changes.map((change) => change.path);
  if (new Set(paths).size !== paths.length) throw new Error("Duplicate changed path");
  // Evaluate the complete input: an early native path must not hide a later analysis failure.
  const plans = changes.map(classifyChange);
  for (const plan of [
    "full",
    "native-bundle",
    "native-code",
    "portable-rust",
    "frontend-only",
  ] as const)
    if (plans.includes(plan)) return plan;
  throw new Error("Incomplete CI plan");
}

export function requireCiResults(
  plan: string,
  portable: string,
  common: string,
  native: string,
): void {
  if (!Object.hasOwn(CI_PLANS, plan)) throw new Error("Unknown CI plan");
  const expected = CI_PLANS[plan as CiPlan];
  if (
    portable !== "success" ||
    common !== (expected.common ? "success" : "skipped") ||
    native !== (expected.native === "none" ? "skipped" : "success")
  )
    throw new Error(`Incomplete CI result: ${plan}/${portable}/${common}/${native}`);
}
