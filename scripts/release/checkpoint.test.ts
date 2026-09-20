import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { GitHub } from "./github.ts";
import { verifyReviewedCheckpoint } from "./checkpoint.ts";
import type { AdmittedRelease } from "./admission.ts";

vi.mock("node:fs", async (original) => ({
  ...(await original<typeof import("node:fs")>()),
  readFileSync: vi.fn<typeof readFileSync>(),
}));
const hash = (bytes: Buffer) => createHash("sha256").update(bytes).digest("hex");
const source = "a9b261e084fd2f96fc8745c725525d17a7e56d91";
const admitted: AdmittedRelease = {
  tag: "v0.5.0",
  version: "0.5.0",
  source,
  pullRequest: 104,
  releaseId: 392344988,
  legacy: false,
  draft: true,
};
function fixture() {
  const dmg = Buffer.from("unit test distribution bytes");
  const checksum = Buffer.from(`${hash(dmg)}  Lens_0.5.0_aarch64.dmg\n`);
  const receipt = {
    schema: 2,
    repository: "japboy/lens",
    tag: admitted.tag,
    version: admitted.version,
    source,
    controller: source,
    runId: "35496202983",
    runAttempt: "1",
    verificationAttempt: "1",
    workflow: ".github/workflows/release-please.yml",
    pullRequest: 104,
    releaseId: admitted.releaseId,
    artifactId: "10601590488",
    assets: [
      { name: "Lens_0.5.0_aarch64.dmg", size: dmg.length, sha256: hash(dmg) },
      { name: "SHA256SUMS", size: checksum.length, sha256: hash(checksum) },
    ],
  };
  const receiptBytes = Buffer.from(JSON.stringify(receipt));
  const bytes = [dmg, checksum, receiptBytes];
  const pins = [
    ...receipt.assets,
    { name: "release-receipt.json", size: receiptBytes.length, sha256: hash(receiptBytes) },
  ].map((asset, index) => ({ ...asset, id: index + 1 }));
  const gate = {
    id: 106040694548,
    name: "release / release-verification",
    completedAt: "2026-09-20T07:21:20Z",
  };
  vi.mocked(readFileSync).mockReturnValue(
    JSON.stringify({ schema: 1, receipt, assets: pins, gate }),
  );
  const release = {
    id: admitted.releaseId,
    tag_name: admitted.tag,
    target_commitish: source,
    draft: true,
    immutable: false,
    prerelease: false,
  };
  const run = {
    id: Number(receipt.runId),
    run_attempt: 1,
    head_sha: source,
    path: receipt.workflow,
    event: "push",
    head_branch: "main",
    repository: { full_name: "japboy/lens" },
    head_repository: { full_name: "japboy/lens" },
  };
  const jobs = [
    {
      id: gate.id,
      name: gate.name,
      completed_at: gate.completedAt,
      status: "completed",
      conclusion: "success",
    },
  ];
  const assets = pins.map((asset) => ({
    ...asset,
    digest: `sha256:${asset.sha256}`,
    state: "uploaded",
  }));
  const ref = { object: { type: "commit", sha: source } };
  const api: GitHub = {
    request: async <T>(path: string, method = "GET"): Promise<T> => {
      expect(method).toBe("GET");
      if (path === `/git/ref/tags/${admitted.tag}`) return ref as T;
      if (path === `/releases/${admitted.releaseId}`) return release as T;
      if (path === `/actions/runs/${receipt.runId}/attempts/1`) return run as T;
      if (path === `/actions/runs/${receipt.runId}/attempts/1/jobs?per_page=100&page=1`)
        return { jobs } as T;
      if (path === `/releases/${admitted.releaseId}/assets?per_page=100&page=1`) return assets as T;
      throw new Error(`Unexpected API access ${path}`);
    },
    download: async (path) => bytes[Number(path.split("/").at(-1)) - 1]!,
    upload: async () => {
      throw new Error("Writes forbidden");
    },
  };
  return { api, release, run, jobs, assets, bytes, ref, receipt };
}
beforeEach(() => vi.clearAllMocks());
describe("reviewed v0.5.0 checkpoint", () => {
  it("verifies exact preserved assets without the missing Actions artifact", async () => {
    const f = fixture();
    await expect(verifyReviewedCheckpoint(f.api, admitted, "japboy/lens")).resolves.toEqual(
      f.receipt,
    );
  });
  it("does not apply to another version", async () => {
    const f = fixture();
    await expect(
      verifyReviewedCheckpoint(f.api, { ...admitted, tag: "v0.5.1" }, "japboy/lens"),
    ).resolves.toBeUndefined();
  });
  it.each(["source", "releaseId", "pullRequest", "legacy"] as const)(
    "rejects changed admission %s",
    async (key) => {
      const f = fixture();
      const changed = {
        ...admitted,
        [key]: key === "legacy" ? true : "different",
      } as AdmittedRelease;
      await expect(verifyReviewedCheckpoint(f.api, changed, "japboy/lens")).rejects.toThrow(
        "identity conflict",
      );
    },
  );
  it.each([
    "partial",
    "extra",
    "id",
    "digest",
    "bytes",
    "tag",
    "controller",
    "gate",
    "gate-id",
    "attempt",
    "repository",
  ])("rejects %s conflict", async (kind) => {
    const f = fixture();
    if (kind === "partial") f.assets.pop();
    if (kind === "extra") f.assets.push(f.assets[0]!);
    if (kind === "id") f.assets[0]!.id++;
    if (kind === "digest") f.assets[0]!.digest = "sha256:wrong";
    if (kind === "bytes") f.bytes[0] = Buffer.from("corrupt");
    if (kind === "tag") f.ref.object.sha = "b".repeat(40);
    if (kind === "controller") f.run.head_sha = "b".repeat(40);
    if (kind === "gate") f.jobs[0]!.conclusion = "failure";
    if (kind === "gate-id") f.jobs[0]!.id++;
    if (kind === "attempt") f.run.run_attempt = 2;
    if (kind === "repository") f.run.head_repository.full_name = "foreign/lens";
    await expect(verifyReviewedCheckpoint(f.api, admitted, "japboy/lens")).rejects.toThrow(
      /checkpoint/u,
    );
  });
  it("requires immutable state if already published", async () => {
    const f = fixture();
    f.release.draft = false;
    const published = { ...admitted, draft: false };
    await expect(verifyReviewedCheckpoint(f.api, published, "japboy/lens")).rejects.toThrow(
      "release/tag conflict",
    );
    f.release.immutable = true;
    await expect(verifyReviewedCheckpoint(f.api, published, "japboy/lens")).resolves.toEqual(
      f.receipt,
    );
  });
});
