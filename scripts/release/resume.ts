import type { GitHub, Request } from "./github.ts";
import { BUNDLE_NAME } from "./recovery-bundle.ts";
import { pages } from "./github.ts";
import { releaseByTag } from "./admission.ts";
import type { AdmittedRelease } from "./admission.ts";
import { sha256 } from "./artifact.ts";
import type { Asset } from "./artifact.ts";
import {
  parseReceipt,
  positiveId,
  RECEIPT_NAME,
  RELEASE_WORKFLOW,
  requireVerificationAttempt,
} from "./receipt.ts";
import type { BuildIdentity, ReleaseReceipt } from "./receipt.ts";

export type RemoteAsset = {
  id: number;
  name: string;
  size: number;
  state: string;
  digest?: string | null;
};
type WorkflowRun = {
  id: number;
  event: string;
  head_sha: string;
  head_branch: string;
  path: string;
  run_attempt: number;
  repository: { full_name: string };
  head_repository: { full_name: string };
};
export type OriginalArtifact = {
  id: number;
  name: string;
  expired: boolean;
  workflow_run: { id: number; head_sha: string; head_branch: string };
};
export type ProvenancePolicy = { repository: string; admitController: (sha: string) => void };

async function artifactList(request: Request, tag: string): Promise<OriginalArtifact[]> {
  const result: OriginalArtifact[] = [];
  for (let page = 1; ; page++) {
    const response = await request<{ artifacts: OriginalArtifact[] }>(
      `/actions/artifacts?name=${encodeURIComponent(`release-${tag}`)}&per_page=100&page=${page}`,
    );
    if (!Array.isArray(response.artifacts)) throw new Error("Invalid artifact list");
    result.push(...response.artifacts);
    if (response.artifacts.length < 100) return result;
  }
}
export async function verifyBuildProvenance(
  request: Request,
  identity: BuildIdentity & { verificationAttempt: string },
  artifactId: string,
  policy: ProvenancePolicy,
): Promise<void> {
  requireVerificationAttempt(identity);
  if (identity.repository !== policy.repository || !positiveId(artifactId))
    throw new Error("Artifact repository/ID conflict");
  policy.admitController(identity.controller);
  const artifact = await request<OriginalArtifact>(`/actions/artifacts/${artifactId}`);
  if (
    String(artifact.id) !== artifactId ||
    artifact.name !== `release-${identity.tag}` ||
    artifact.expired ||
    String(artifact.workflow_run.id) !== identity.runId ||
    artifact.workflow_run.head_sha !== identity.controller ||
    artifact.workflow_run.head_branch !== "main"
  )
    throw new Error("Original artifact workflow provenance mismatch");
  await verifyBuildRun(request, identity, policy);
}
export async function verifyBuildRun(
  request: Request,
  identity: BuildIdentity & { verificationAttempt: string },
  policy: ProvenancePolicy,
  signingAttempt?: string,
): Promise<void> {
  requireVerificationAttempt(identity);
  if (identity.repository !== policy.repository) throw new Error("Build repository conflict");
  policy.admitController(identity.controller);
  // A partial rerun may verify an earlier successful native build. Both attempts
  // must belong to the same trusted controller run; only the later gate authorizes promotion.
  for (const attempt of new Set([
    identity.runAttempt,
    identity.verificationAttempt,
    ...(signingAttempt ? [signingAttempt] : []),
  ])) {
    const run = await request<WorkflowRun>(`/actions/runs/${identity.runId}/attempts/${attempt}`);
    if (
      String(run.id) !== identity.runId ||
      String(run.run_attempt) !== attempt ||
      run.head_sha !== identity.controller ||
      run.path !== RELEASE_WORKFLOW ||
      run.head_branch !== "main" ||
      !["push", "workflow_dispatch"].includes(run.event) ||
      run.repository.full_name !== policy.repository ||
      run.head_repository.full_name !== policy.repository
    )
      throw new Error("Original artifact workflow provenance mismatch");
  }
  const jobs: { name: string; conclusion: string | null }[] = [];
  for (let page = 1; ; page++) {
    const response = await request<{ jobs: { name: string; conclusion: string | null }[] }>(
      `/actions/runs/${identity.runId}/attempts/${identity.verificationAttempt}/jobs?per_page=100&page=${page}`,
    );
    if (!Array.isArray(response.jobs)) throw new Error("Invalid verification jobs response");
    jobs.push(...response.jobs);
    if (response.jobs.length < 100) break;
  }
  const verification = jobs.filter((job) =>
    ["release-verification", "release / release-verification"].includes(job.name),
  );
  if (verification.length !== 1 || verification[0]!.conclusion !== "success")
    throw new Error("Original build lacks successful release verification");
}
export async function verifyRemoteAsset(
  api: GitHub,
  remote: RemoteAsset,
  expected: Asset,
): Promise<void> {
  if (
    !Number.isSafeInteger(remote.id) ||
    remote.id < 1 ||
    remote.name !== expected.name ||
    remote.state !== "uploaded" ||
    remote.size !== expected.size ||
    (remote.digest != null && remote.digest !== `sha256:${expected.sha256}`)
  )
    throw new Error(`Release asset metadata conflict: ${expected.name}`);
  const bytes = await api.download(`/releases/assets/${remote.id}`);
  if (bytes.length !== expected.size || sha256(bytes) !== expected.sha256)
    throw new Error(`Release asset bytes conflict: ${expected.name}`);
}
export async function verifyPublishedRelease(
  api: GitHub,
  admitted: AdmittedRelease,
  repository: string,
): Promise<ReleaseReceipt> {
  const release = await releaseByTag(api.request, admitted.tag);
  if (
    release.id !== admitted.releaseId ||
    release.draft ||
    !release.immutable ||
    release.target_commitish !== admitted.source
  )
    throw new Error("Expected immutable published release with matching source");
  const remote = await pages<RemoteAsset>(api.request, `/releases/${release.id}/assets`);
  const receipts = remote.filter((asset) => asset.name === RECEIPT_NAME);
  if (receipts.length !== 1) throw new Error("Published release receipt/inventory missing");
  const receiptAsset = receipts[0]!;
  if (
    !Number.isSafeInteger(receiptAsset.id) ||
    receiptAsset.id < 1 ||
    receiptAsset.size > 64 * 1024
  )
    throw new Error("Invalid receipt asset");
  const bytes = await api.download(`/releases/assets/${receiptAsset.id}`);
  await verifyRemoteAsset(api, receiptAsset, {
    name: RECEIPT_NAME,
    size: bytes.length,
    sha256: sha256(bytes),
  });
  const receipt = parseReceipt(bytes, admitted, repository);
  const expectedAssets =
    receipt.schema === 3 ? [...receipt.assets, receipt.bundle!] : receipt.assets;
  if (
    remote.length !== expectedAssets.length + 1 ||
    remote.some(
      (asset) =>
        asset.name !== RECEIPT_NAME &&
        !expectedAssets.some((expected) => expected.name === asset.name),
    )
  )
    throw new Error("Published release receipt/inventory missing");
  for (const expected of expectedAssets) {
    const matches = remote.filter((asset) => asset.name === expected.name);
    if (matches.length !== 1) throw new Error("Published release assets conflict");
    await verifyRemoteAsset(api, matches[0]!, expected);
  }
  // Validate the checksum file's contents, not just its receipt-provided digest.
  const checksum = remote.find((asset) => asset.name === "SHA256SUMS")!;
  if (
    (await api.download(`/releases/assets/${checksum.id}`)).toString("utf8") !==
    `${receipt.assets[0]!.sha256}  ${receipt.assets[0]!.name}\n`
  )
    throw new Error("Published checksum content conflict");
  return receipt;
}
export type ResumeState =
  | { state: "build" }
  | { state: "reuse"; artifactId: string; runId: string }
  | { state: "durable" }
  | { state: "published"; receipt: ReleaseReceipt }
  | { state: "legacy-published" };

export async function resumeRelease(
  api: GitHub,
  admitted: AdmittedRelease,
  repository: string,
): Promise<ResumeState> {
  const release = await releaseByTag(api.request, admitted.tag);
  if (release.id !== admitted.releaseId || release.target_commitish !== admitted.source)
    throw new Error("Release changed during admission");
  if (!release.draft) {
    if (admitted.legacy) return { state: "legacy-published" };
    return { state: "published", receipt: await verifyPublishedRelease(api, admitted, repository) };
  }
  if (admitted.legacy)
    throw new Error("Finish the legacy draft with the legacy release workflow before cutover");
  const assets = await pages<RemoteAsset>(api.request, `/releases/${release.id}/assets`);
  const bundles = assets.filter((asset) => asset.name === BUNDLE_NAME);
  if (bundles.length > 1) throw new Error("Duplicate recovery bundle");
  if (bundles.length === 1) return { state: "durable" };
  const artifacts = (await artifactList(api.request, admitted.tag)).filter(
    (item) => item.name === `release-${admitted.tag}` && !item.expired,
  );
  if (artifacts.length > 1) throw new Error("Multiple original artifacts require reconciliation");
  if (artifacts.length === 1) {
    const artifact = artifacts[0]!;
    if (
      !Number.isSafeInteger(artifact.id) ||
      artifact.id < 1 ||
      !Number.isSafeInteger(artifact.workflow_run.id) ||
      artifact.workflow_run.id < 1
    )
      throw new Error("Invalid original artifact identity");
    // This is only a download selection. Full provenance and bytes are admitted by the publisher.
    return {
      state: "reuse",
      artifactId: String(artifact.id),
      runId: String(artifact.workflow_run.id),
    };
  }
  if (assets.length)
    throw new Error("Unpublished assets exist without the original artifact; refusing rebuild");
  return { state: "build" };
}
