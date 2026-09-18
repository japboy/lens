import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { planChanges } from "../ci-plan.ts";
const release = readFileSync(".github/workflows/release.yml", "utf8");
const automation = readFileSync(".github/workflows/release-please.yml", "utf8");
const native = readFileSync(".github/workflows/native-quality.yml", "utf8");

describe("release workflow authority and recovery", () => {
  it("dispatches through one main controller without tag-push races", () => {
    expect(automation).toContain("workflow_dispatch:");
    expect(automation).toContain('test "$REF" = refs/heads/main');
    expect(automation).toContain("node scripts/release/cli.ts generate");
    expect(automation).not.toContain("refresh-lock");
    expect(automation).not.toContain("release-please-action@");
    expect(release).toContain("workflow_call:");
    expect(release).not.toContain("tags:");
    expect(release).not.toContain("workflow_dispatch:");
    expect(release).toContain("group: release-${{ inputs.tag }}");
  });
  it("uses the controller for privilege and source for read-only builds", () => {
    expect(release.match(/contents: write/gu)).toHaveLength(2);
    expect(release).toContain("ref: ${{ inputs.controller_sha }}");
    expect(release).toContain("ref: ${{ needs.preflight.outputs.source_sha }}");
    expect(native).toContain("node target/release-controller/scripts/release/cli.ts package");
    expect(native).toContain("persist-credentials: false");
    expect(native).not.toContain("secrets.");
    expect(automation).not.toContain("secrets: inherit");
    const policy = release
      .split("  publication-policy:\n")[1]!
      .split("\n  release-verification:")[0]!;
    expect(policy).toContain("permission-administration: read");
    expect(policy).not.toContain("checkout@");
  });
  it("promotes canonical artifacts only after all verification, and binds cross-run downloads", () => {
    expect(native).toContain("name: release-candidate-");
    expect(native).not.toContain("name: release-${{ inputs.release_tag }}");
    expect(release).toContain("needs: [preflight, native, release-verification]");
    expect(release).toContain("name: release-${{ inputs.tag }}");
    expect(release).toContain("name: release-verification");
    expect(release).toContain("node scripts/release/cli.ts promote");
    expect(release).toContain('test "$RESULTS" = success:success:success');
    expect(release).toContain(
      "run-id: ${{ needs.preflight.outputs.artifact_run_id || github.run_id }}",
    );
    expect(release).toContain("github-token: ${{ github.token }}");
    expect(release).not.toContain("overwrite: true");
    expect(release).toContain("needs.verified-artifact.result == 'success'");
  });
  it.each([
    "release-please-config.json",
    ".release-please-manifest.json",
    "CHANGELOG.md",
    "scripts/release/lifecycle.ts",
    ".github/workflows/release.yml",
  ])("requires full verification for %s", (path) => {
    expect(planChanges([{ path, before: "old", after: "new" }])).toBe("full");
  });
});
