import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { planChanges } from "../ci-plan.ts";

const release = readFileSync(".github/workflows/release.yml", "utf8");
const automation = readFileSync(".github/workflows/release-please.yml", "utf8");
const native = readFileSync(".github/workflows/native-quality.yml", "utf8");

describe("release workflow source, credentials and retry boundaries", () => {
  it("requires exact tags and all verification before publisher execution", () => {
    expect(release).toMatch(/tags: \[["']v\*["']\]/u);
    expect(release).not.toContain("workflow_dispatch:");
    expect(release).toContain("node scripts/release/cli.ts preflight");
    expect(release).toContain("ref: ${{ needs.preflight.outputs.source_sha }}");
    expect(release).toContain("needs.portable.result == 'success'");
    expect(release).toContain("needs.common.result == 'success'");
    expect(release).toContain("needs.native.result == 'success'");
    expect(release).not.toContain("scripts/ci-diff.ts");
    expect(release).toContain("cancel-in-progress: false");
  });
  it("preserves prerequisite artifact outputs when rerunning only a failed consumer", () => {
    expect(release).toContain("frontend_artifact: ${{ steps.frontend.outputs.name }}");
    expect(
      release.match(/frontend_artifact: \$\{\{ needs.portable.outputs.frontend_artifact \}\}/gu),
    ).toHaveLength(2);
    expect(release).toContain(
      "artifact-ids: ${{ needs.preflight.outputs.artifact_id || needs.native.outputs.artifact_id }}",
    );
    expect(release).toContain("needs.preflight.outputs.state == 'build' && 'bundle' || 'code'");
    expect(native).toContain("retention-days: 30");
    expect(native).not.toContain("overwrite: true");
  });
  it("isolates administration read from builds and gives contents write only to publication", () => {
    const policy = release.split("  publication-policy:\n")[1]!.split("\n  publish:")[0]!;
    expect(policy).toContain("permission-administration: read");
    expect(policy).not.toContain("checkout@");
    expect(policy).not.toContain("node scripts/");
    expect(release.match(/contents: write/gu)).toHaveLength(1);
    expect(release).toContain("IMMUTABILITY_RESULT: ${{ needs.publication-policy.result }}");
    expect(automation).toContain("skip-github-release: true");
    expect(automation).toContain("permission-contents: write");
    expect(automation).toContain("permission-pull-requests: write");
    expect(automation).not.toContain("permission-administration:");
    expect(native).not.toContain("secrets.");
    expect(native).not.toContain("create-github-app-token");
  });
  it.each([
    "release-please-config.json",
    ".release-please-manifest.json",
    "CHANGELOG.md",
    "apps/desktop/src-tauri/tauri.conf.json",
    "apps/desktop/src-tauri/tauri.release.conf.json",
    "scripts/release/publish.ts",
    ".github/workflows/release.yml",
  ])("routes release control through full PR verification: %s", (path) => {
    expect(planChanges([{ path, before: "old", after: "new" }])).toBe("full");
  });
});
