import { describe, expect, it } from "vitest";
import { isTypescriptTestSupport, testSupportImportViolations } from "./typescript-test-paths.ts";

describe("recursive TypeScript test ownership", () => {
  it.each([
    "tests/build/deep/integrity.ts",
    "apps/desktop/tests/mcp-apps/presentation.ts",
    "packages/adapter-mcp-apps-host/tests/protocol/handshake.ts",
    "packages/adapter-mcp-apps-view/tests/rendering/document-assets.ts",
    "packages/adapter-math-renderer/tests/assets/public-math.ts",
  ])("classifies collaboration cases as test-only: %s", (path) => {
    expect(isTypescriptTestSupport(path)).toBe(true);
  });
  it.each([
    "tests/build/helpers/seal.ts",
    "tests/build/fixtures/document.ts",
    "tests/build/seal.test-helper.ts",
    "tests/build/document.fixture.ts",
  ])("classifies helpers and fixtures as test-only: %s", (path) => {
    expect(isTypescriptTestSupport(path)).toBe(true);
  });
  it("distinguishes adjacent unit tests from runtime modules", () => {
    expect(isTypescriptTestSupport("packages/adapter-mcp-apps-host/src/controller.test.ts")).toBe(
      true,
    );
    expect(isTypescriptTestSupport("packages/adapter-mcp-apps-host/src/controller.ts")).toBe(false);
  });
  it("allows only the explicit qualification entry to import its fixture", () => {
    const fixture = "apps/desktop/tests/fixtures/media-parity.ts";
    const source = 'import { mediaFixture } from "../tests/fixtures/media-parity";';
    expect(
      testSupportImportViolations(
        "apps/desktop/src/media-qualification.ts",
        source,
        new Set([fixture]),
      ),
    ).toEqual([]);
    expect(
      testSupportImportViolations("apps/desktop/src/overlay.ts", source, new Set([fixture])),
    ).toEqual(["apps/desktop/src/overlay.ts imports test-only ../tests/fixtures/media-parity"]);
  });
  it("rejects a runtime import of the qualification entry itself", () => {
    expect(
      testSupportImportViolations(
        "apps/desktop/src/overlay.ts",
        'import "./media-qualification";',
        new Set(["apps/desktop/src/media-qualification.ts"]),
      ),
    ).not.toEqual([]);
  });
});
