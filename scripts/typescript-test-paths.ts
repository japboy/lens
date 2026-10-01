import { posix } from "node:path";

/** Test ownership is independent of the production source entry graph. */
export const TEST_DISCOVERY_EXCLUDES = [
  "**/helpers/**",
  "**/fixtures/**",
  "**/*.test-helper.ts",
  "**/*.fixture.ts",
] as const;

const TEST_OWNERS =
  /^(?:tests\/|apps\/desktop\/tests\/|packages\/(?:adapter-mcp-apps-host|adapter-mcp-apps-view|adapter-math-renderer)\/tests\/)/u;

export function isTypescriptTestSupport(path: string): boolean {
  return (
    /\.(?:ts|tsx|js|mjs)$/u.test(path) &&
    (path === "apps/desktop/src/media-qualification.ts" ||
      TEST_OWNERS.test(path) ||
      /\.(?:test|test-helper|fixture)\.tsx?$/u.test(path))
  );
}

/** Conservative literal-reference admission, not a full JavaScript parser. */
export function testSupportImportViolations(
  path: string,
  source: string,
  testPaths: ReadonlySet<string>,
): string[] {
  if (isTypescriptTestSupport(path)) return [];
  const violations: string[] = [];
  for (const match of source.matchAll(/["'`]([^"'`\r\n]+)["'`]/gu)) {
    const reference = match[1]!;
    if (!reference.startsWith(".")) continue;
    const target = posix.normalize(posix.join(posix.dirname(path), reference));
    const candidates = [target, `${target}.ts`, target.replace(/\.js$/u, ".ts")];
    if (candidates.some((candidate) => testPaths.has(candidate)))
      violations.push(`${path} imports test-only ${reference}`);
  }
  return violations;
}
