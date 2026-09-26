import { portableSourceViolations } from "./rust-source-boundaries.ts";
import { rustDeclarationSurface } from "./rust-source-surface.ts";
import { NATIVE_CODE_INPUTS } from "./ci-native-inputs.ts";

export type VerificationRequirements =
  | { sharedRust: false; macos: "none" }
  | { sharedRust: true; macos: "none" | "code" | "app" | "dmg" };
export const VERIFICATION_REQUIREMENTS = [
  { sharedRust: false, macos: "none" },
  { sharedRust: true, macos: "none" },
  { sharedRust: true, macos: "code" },
  { sharedRust: true, macos: "app" },
  { sharedRust: true, macos: "dmg" },
] as const satisfies readonly VerificationRequirements[];
export type Change = { path: string; before: string | null; after: string | null };
export type ChangeRequirement = {
  path: string;
  ruleId: string;
  reason: string;
  requirements: VerificationRequirements;
};

// Exact reviewed test owners, not an exemption for every test-looking filename.
export const FRONTEND_TEST_INPUTS = [
  "apps/desktop/tooling/build-paths.test.ts",
  "apps/desktop/tooling/html-math-assets.test.ts",
  "apps/desktop/tooling/math-assets.test.ts",
  "apps/desktop/tests/styles.ts",
  "apps/desktop/tests/overlay-close.ts",
] as const;
const FRONTEND_CONTRACTS = new Set([
  "apps/desktop/src/types.ts",
  "apps/desktop/src/agent-prompt-template.ts",
  "apps/desktop/src/output-media.ts",
  "apps/desktop/src/presentation-context.ts",
  "apps/desktop/src/application/webview-port.ts",
  "apps/desktop/src/application/accessibility-permission-controller.ts",
]);

export function validateRequirements(value: unknown): VerificationRequirements {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    throw new Error("Invalid verification requirements");
  const candidate = value as Record<string, unknown>;
  const state = VERIFICATION_REQUIREMENTS.find(
    (entry) => entry.sharedRust === candidate.sharedRust && entry.macos === candidate.macos,
  );
  if (!state || Object.keys(candidate).sort().join(",") !== "macos,sharedRust")
    throw new Error("Invalid verification requirements");
  return { ...state };
}

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

export function classifyChange(change: Change): ChangeRequirement {
  const { path } = change;
  validateChangedPath(path);
  const require = (
    ruleId: string,
    reason: string,
    requirements: VerificationRequirements,
  ): ChangeRequirement => ({
    path,
    ruleId,
    reason,
    requirements: validateRequirements(requirements),
  });
  const [frontend, shared, code, app, dmg] = VERIFICATION_REQUIREMENTS;
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
    return require("control-plane", "Build, dependency or verification policy requires complete verification", dmg);
  if (NATIVE_CODE_INPUTS.has(path))
    return require("native-input", "Rust includes or build scripts consume this input", code);
  if (FRONTEND_CONTRACTS.has(path))
    return require("frontend-native-contract", "Frontend contract is shared with the native implementation", code);
  if (
    path.startsWith("packages/adapter-platform-macos/") ||
    path.startsWith("packages/adapter-output-mcp/") ||
    path.startsWith("apps/desktop/src-tauri/src/native/") ||
    path === "apps/desktop/src-tauri/src/lib.rs" ||
    path === "apps/desktop/src-tauri/src/main.rs"
  )
    return require("native-application", "Native entry point or adapter requires application verification", app);
  if (
    path.startsWith("packages/port-platform/") ||
    /^apps\/desktop\/src-tauri\/src\/.+\.rs$/u.test(path)
  )
    return require("native-code", "Native code or platform interface requires macOS code verification", code);
  if (path.startsWith("apps/desktop/src-tauri/"))
    return require("native-resource", "Native build input or resource requires application verification", app);
  if (/^packages\/(?:domain|usecase)\/src\/.+\.rs$/u.test(path)) {
    if (change.before === null || change.after === null)
      return require("shared-rust-file-set", "Added or deleted shared Rust file requires native consumers", code);
    for (const source of [change.before, change.after]) {
      const violations = portableSourceViolations(source);
      if (violations.length)
        throw new Error(`Portable boundary failure: ${path}: ${violations.join("; ")}`);
    }
    return rustDeclarationSurface(change.before) === rustDeclarationSurface(change.after)
      ? require("shared-rust-body", "Only portable Rust implementation bodies changed", shared)
      : require("shared-rust-surface", "Shared Rust declarations changed and require native consumers", code);
  }
  if ((FRONTEND_TEST_INPUTS as readonly string[]).includes(path))
    return require("frontend-test", "Reviewed test-only frontend owner", frontend);
  if (
    /^apps\/desktop\/src\/.+\.(?:html|ts|css|svg)$/u.test(path) ||
    /^apps\/desktop\/(?:public|tests\/fixtures)\/.+\.(?:html|css|ts|js|json|svg|png|jpg|webp)$/u.test(
      path,
    ) ||
    path === "README.md"
  )
    return require("frontend-source", "Frontend source, fixture or repository readme", frontend);
  return require("unreviewed-input", "Input has no reviewed narrower verification owner", dmg);
}

export function planChanges(changes: readonly Change[]): {
  requirements: VerificationRequirements;
  reasons: ChangeRequirement[];
} {
  if (!changes.length) return { requirements: { ...VERIFICATION_REQUIREMENTS[4] }, reasons: [] };
  const paths = changes.map((change) => change.path);
  if (new Set(paths).size !== paths.length) throw new Error("Duplicate changed path");
  // Evaluate all paths before joining: an earlier complete requirement cannot hide a parser failure.
  const reasons = changes
    .map(classifyChange)
    .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  const index = reasons.reduce(
    (maximum, { requirements }) =>
      Math.max(
        maximum,
        VERIFICATION_REQUIREMENTS.findIndex(
          (state) =>
            state.sharedRust === requirements.sharedRust && state.macos === requirements.macos,
        ),
      ),
    0,
  );
  return { requirements: { ...VERIFICATION_REQUIREMENTS[index]! }, reasons };
}
