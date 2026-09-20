import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parse } from "yaml";
import { planChanges } from "../ci-plan.ts";
const release = readFileSync(".github/workflows/release.yml", "utf8");
const automation = readFileSync(".github/workflows/release-please.yml", "utf8");
const cli = readFileSync("scripts/release/cli.ts", "utf8");
const native = readFileSync(".github/workflows/native-quality.yml", "utf8");

describe("release workflow authority and recovery", () => {
  it("attests only after verification and grants signing authority only to promotion", () => {
    const workflow = parse(release);
    expect(workflow.permissions["id-token"]).toBeUndefined();
    expect(workflow.permissions.attestations).toBeUndefined();
    expect(workflow.jobs["verified-artifact"].permissions).toMatchObject({
      "id-token": "write",
      attestations: "write",
    });
    expect(workflow.jobs.publish.permissions.attestations).toBe("read");
    expect(workflow.jobs.publish.permissions["id-token"]).toBeUndefined();
    const steps = workflow.jobs["verified-artifact"].steps as { run?: string; uses?: string }[];
    const promotion = steps.findIndex((step) => step.run?.endsWith(" promote"));
    const attestation = steps.findIndex((step) => step.uses?.startsWith("actions/attest@"));
    const upload = steps.findIndex((step) => step.uses?.startsWith("actions/upload-artifact@"));
    expect(promotion).toBeGreaterThanOrEqual(0);
    expect(attestation).toBeGreaterThan(promotion);
    expect(upload).toBeGreaterThan(attestation);
  });
  it("separates live policy credentials from the release writer and child processes", () => {
    const steps = parse(automation).jobs.reconcile.steps as {
      id?: string;
      with?: Record<string, string>;
      env?: Record<string, string>;
    }[];
    const writer = steps.find((step) => step.id === "app")!.with!;
    const policy = steps.find((step) => step.id === "policy")!.with!;
    const permissions = (inputs: Record<string, string>) =>
      Object.fromEntries(Object.entries(inputs).filter(([key]) => key.startsWith("permission-")));
    expect(permissions(writer)).toEqual({
      "permission-contents": "write",
      "permission-pull-requests": "write",
    });
    expect(permissions(policy)).toEqual({ "permission-administration": "write" });
    expect(policy.repositories).toBe("${{ github.event.repository.name }}");
    expect(steps.filter((step) => step.env?.GH_POLICY_TOKEN).map((step) => step.id)).toEqual([
      "reconcile",
    ]);
    expect(steps.find((step) => step.id === "reconcile")!.env?.GH_POLICY_TOKEN).toBe(
      "${{ steps.policy.outputs.token }}",
    );
    const generation = cli.split('mode === "generate"')[1]!.split('mode === "delta"')[0]!;
    expect(generation).toContain("delete process.env.GH_POLICY_TOKEN");
    expect(generation.indexOf("delete process.env.GH_POLICY_TOKEN")).toBeLessThan(
      generation.indexOf("await generate("),
    );
  });
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
  it("retains the successful gate attempt when only artifact promotion is rerun", () => {
    expect(release).toContain("verification_attempt: ${{ steps.verified.outputs.attempt }}");
    expect(release).toContain('echo "attempt=$GITHUB_RUN_ATTEMPT" >> "$GITHUB_OUTPUT"');
    expect(release).toContain(
      "VERIFICATION_ATTEMPT: ${{ needs.release-verification.outputs.verification_attempt }}",
    );
    const promotion = cli.split('mode === "promote"')[1]!.split('mode === "package"')[0]!;
    expect(promotion).toContain('env("VERIFICATION_ATTEMPT")');
    expect(promotion.split("writeFileSync(")[0]).not.toContain('env("GITHUB_RUN_ATTEMPT")');
    expect(promotion).toContain('manifest, env("GITHUB_RUN_ATTEMPT")');
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
