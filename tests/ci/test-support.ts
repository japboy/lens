import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { FRONTEND_TEST_INPUTS } from "../../scripts/ci-plan.ts";
import {
  isTypescriptTestSupport,
  testSupportImportViolations,
} from "../../scripts/typescript-test-paths.ts";

const root = fileURLToPath(new URL("../..", import.meta.url));
const tracked = execFileSync(
  "git",
  ["ls-files", "-z", "--cached", "--others", "--exclude-standard"],
  { cwd: root, encoding: "utf8" },
)
  .split("\0")
  .filter((path) => path && existsSync(resolve(root, path)));

describe("CI and production dependency test-support separation", () => {
  it("keeps exact frontend test exceptions outside production TypeScript imports", () => {
    const tests = new Set(
      tracked.filter(
        (path) =>
          isTypescriptTestSupport(path) ||
          (FRONTEND_TEST_INPUTS as readonly string[]).includes(path),
      ),
    );
    const violations = tracked
      .filter((path) => /\.(?:ts|tsx|js|mjs)$/u.test(path))
      .flatMap((path) =>
        testSupportImportViolations(path, readFileSync(resolve(root, path), "utf8"), tests),
      );
    expect(violations).toEqual([]);
  });
});
