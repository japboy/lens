import { readFileSync } from "node:fs";
import type { AdmittedRelease } from "./admission.ts";
import { sha256 } from "./artifact.ts";
import type { GitHub } from "./github.ts";
import { pages } from "./github.ts";
import { parseReceipt, RECEIPT_NAME } from "./receipt.ts";
import type { ReleaseReceipt } from "./receipt.ts";

type PinnedAsset = { id: number; name: string; size: number; sha256: string };
type Checkpoint = {
  schema: 1;
  receipt: ReleaseReceipt;
  assets: PinnedAsset[];
  gate: { id: number; name: string; completedAt: string };
};

/** One reviewed historical exception, never an input-controlled trust registry.
 * This verifies preserved bytes; it does not reconstruct or attest the lost artifact. */
export async function verifyReviewedCheckpoint(
  api: GitHub,
  admitted: AdmittedRelease,
  repository: string,
): Promise<ReleaseReceipt | undefined> {
  if (admitted.tag !== "v0.5.0") return undefined;
  const checkpoint = JSON.parse(
    readFileSync(new URL("./checkpoints/v0.5.0.json", import.meta.url), "utf8"),
  ) as Checkpoint;
  const pin = checkpoint.receipt;
  if (
    checkpoint.schema !== 1 ||
    pin.schema !== 2 ||
    repository !== pin.repository ||
    admitted.tag !== pin.tag ||
    admitted.version !== pin.version ||
    admitted.source !== pin.source ||
    admitted.releaseId !== pin.releaseId ||
    admitted.pullRequest !== pin.pullRequest ||
    admitted.legacy
  )
    throw new Error("Reviewed checkpoint identity conflict");
  const ref = await api.request<{ object: { type: string; sha: string } }>(
    `/git/ref/tags/${pin.tag}`,
  );
  const release = await api.request<{
    id: number;
    tag_name: string;
    target_commitish: string;
    draft: boolean;
    immutable?: boolean;
    prerelease: boolean;
  }>(`/releases/${pin.releaseId}`);
  if (
    ref.object.type !== "commit" ||
    ref.object.sha !== pin.source ||
    release.id !== pin.releaseId ||
    release.tag_name !== pin.tag ||
    release.target_commitish !== pin.source ||
    release.prerelease ||
    release.draft !== admitted.draft ||
    (!release.draft && release.immutable !== true)
  )
    throw new Error("Reviewed checkpoint release/tag conflict");
  const run = await api.request<{
    id: number;
    run_attempt: number;
    head_sha: string;
    path: string;
    event: string;
    head_branch: string;
    repository: { full_name: string };
    head_repository: { full_name: string };
  }>(`/actions/runs/${pin.runId}/attempts/${pin.verificationAttempt}`);
  if (
    String(run.id) !== pin.runId ||
    String(run.run_attempt) !== pin.verificationAttempt ||
    run.head_sha !== pin.controller ||
    run.path !== pin.workflow ||
    run.event !== "push" ||
    run.head_branch !== "main" ||
    run.repository.full_name !== repository ||
    run.head_repository.full_name !== repository
  )
    throw new Error("Reviewed checkpoint historical run conflict");
  const jobs: {
    id: number;
    name: string;
    status: string;
    conclusion: string;
    completed_at: string;
  }[] = [];
  for (let page = 1; ; page++) {
    const result = await api.request<{ jobs: typeof jobs }>(
      `/actions/runs/${pin.runId}/attempts/${pin.verificationAttempt}/jobs?per_page=100&page=${page}`,
    );
    if (!Array.isArray(result.jobs)) throw new Error("Invalid historical jobs response");
    jobs.push(...result.jobs);
    if (result.jobs.length < 100) break;
  }
  const gates = jobs.filter((job) => job.name === checkpoint.gate.name);
  if (
    gates.length !== 1 ||
    gates[0]!.id !== checkpoint.gate.id ||
    gates[0]!.status !== "completed" ||
    gates[0]!.conclusion !== "success" ||
    gates[0]!.completed_at !== checkpoint.gate.completedAt
  )
    throw new Error("Reviewed checkpoint historical verification conflict");
  const assets = await pages<{
    id: number;
    name: string;
    size: number;
    digest: string;
    state: string;
  }>(api.request, `/releases/${pin.releaseId}/assets`);
  if (assets.length !== 3 || checkpoint.assets.length !== 3)
    throw new Error("Reviewed checkpoint requires the complete original three assets");
  const downloaded = new Map<string, Buffer>();
  for (const expected of checkpoint.assets) {
    const matches = assets.filter((asset) => asset.name === expected.name);
    const asset = matches[0];
    if (
      matches.length !== 1 ||
      !asset ||
      asset.id !== expected.id ||
      asset.size !== expected.size ||
      asset.digest !== `sha256:${expected.sha256}` ||
      asset.state !== "uploaded"
    )
      throw new Error("Reviewed checkpoint asset metadata conflict");
    const bytes = await api.download(`/releases/assets/${asset.id}`);
    if (bytes.length !== expected.size || sha256(bytes) !== expected.sha256)
      throw new Error("Reviewed checkpoint asset bytes conflict");
    downloaded.set(asset.name, bytes);
  }
  const receipt = parseReceipt(downloaded.get(RECEIPT_NAME)!, admitted, repository);
  for (const key of Object.keys(pin) as (keyof ReleaseReceipt)[]) {
    if (JSON.stringify(receipt[key]) !== JSON.stringify(pin[key]))
      throw new Error("Reviewed checkpoint receipt conflict");
  }
  for (const asset of receipt.assets) {
    const bytes = downloaded.get(asset.name);
    if (!bytes || bytes.length !== asset.size || sha256(bytes) !== asset.sha256)
      throw new Error("Reviewed checkpoint receipt asset conflict");
  }
  const dmg = receipt.assets[0]!;
  if (downloaded.get("SHA256SUMS")!.toString("utf8") !== `${dmg.sha256}  ${dmg.name}\n`)
    throw new Error("Reviewed checkpoint checksum conflict");
  return receipt;
}

export async function verifyCheckpointRelease(
  api: GitHub,
  admitted: AdmittedRelease,
  repository: string,
): Promise<boolean> {
  return Boolean(await verifyReviewedCheckpoint(api, admitted, repository));
}
