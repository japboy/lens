import { describe, expect, it } from "vitest";
import { isTypescriptTestSupport, testSupportImportViolations } from "./typescript-test-paths.ts";

describe("recursive TypeScript test ownership", () => {
  it.each([
    "tests/build/deep/integrity.ts",
    "apps/desktop/tests/mcp-apps/presentation.ts",
    "packages/adapter-mcp-apps-host/tests/protocol/handshake.ts",
    "packages/adapter-mcp-apps-view/tests/rendering/document-assets.ts",
    "packages/adapter-math-renderer/tests/assets/public-math.ts",
    "packages/ui/tests/overlay/model-publication.ts",
    "packages/adapter-lit-prerenderer/tests/generation/source-transaction.ts",
    "apps/ui-preview/tests/scenarios/interaction.ts",
    "vitest.config.ts",
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
  it("does not allow package exports to bypass test-only ownership", () => {
    const source = 'import "ui/test-fixtures/media-parity"; import "ui/test-setup";';
    expect(
      testSupportImportViolations("apps/desktop/src/overlay.ts", source, new Set()),
    ).toHaveLength(2);
    expect(
      testSupportImportViolations("apps/desktop/src/media-qualification.ts", source, new Set()),
    ).toEqual([]);
  });
  it("admits central test setup configuration while rejecting its production consumers", () => {
    const source = 'import "./packages/ui/test-setup.ts";';
    expect(
      testSupportImportViolations(
        "vitest.config.ts",
        source,
        new Set(["packages/ui/test-setup.ts"]),
      ),
    ).toEqual([]);
    expect(
      testSupportImportViolations(
        "apps/desktop/src/main.ts",
        'import "../../../vitest.config.ts";',
        new Set(["vitest.config.ts"]),
      ),
    ).not.toEqual([]);
  });
});
