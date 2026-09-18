import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { GitHub } from "./github.ts";
import { pages } from "./github.ts";
import { releaseByTag } from "./admission.ts";
import type { AdmittedRelease } from "./admission.ts";
import { sha256 } from "./artifact.ts";
import { createReceipt, RECEIPT_NAME, verifyArtifactV2 } from "./receipt.ts";
import { verifyBuildProvenance, verifyPublishedRelease, verifyRemoteAsset } from "./resume.ts";
import type { ProvenancePolicy, RemoteAsset } from "./resume.ts";

export async function publishReleaseV2(
  api: GitHub,
  directory: string,
  admitted: AdmittedRelease,
  provenance: ProvenancePolicy & { artifactId: string; readmit: () => Promise<AdmittedRelease> },
  enabled: boolean,
  immutableVerified: boolean,
): Promise<"draft" | "published" | "already-published"> {
  if (admitted.legacy)
    throw new Error("Legacy releases must retain the legacy publication contract");
  const revalidate = async () => {
    const current = await provenance.readmit();
    if (
      current.tag !== admitted.tag ||
      current.source !== admitted.source ||
      current.version !== admitted.version ||
      current.pullRequest !== admitted.pullRequest ||
      current.releaseId !== admitted.releaseId ||
      current.legacy !== admitted.legacy
    )
      throw new Error("Release admission changed during publication");
  };
  await revalidate();
  const release = await releaseByTag(api.request, admitted.tag);
  if (release.id !== admitted.releaseId || release.target_commitish !== admitted.source)
    throw new Error("Release identity changed before publication");
  if (!release.draft) {
    await verifyPublishedRelease(api, admitted, provenance.repository);
    return "already-published";
  }
  const manifest = verifyArtifactV2(directory, { ...admitted, repository: provenance.repository });
  await verifyBuildProvenance(api.request, manifest, provenance.artifactId, provenance);
  const receipt = createReceipt(manifest, admitted, provenance.artifactId);
  const receiptBytes = Buffer.from(`${JSON.stringify(receipt, null, 2)}\n`);
  const assets = [
    ...manifest.assets.map((asset) => ({
      ...asset,
      bytes: readFileSync(join(directory, asset.name)),
    })),
    {
      name: RECEIPT_NAME,
      size: receiptBytes.length,
      sha256: sha256(receiptBytes),
      bytes: receiptBytes,
    },
  ];
  const reconcile = async (upload: boolean) => {
    const remote = await pages<RemoteAsset>(api.request, `/releases/${release.id}/assets`);
    if (
      new Set(remote.map((asset) => asset.name)).size !== remote.length ||
      remote.some((asset) => !assets.some((expected) => expected.name === asset.name))
    )
      throw new Error("Unexpected release assets");
    // Check every existing file before uploading anything else.
    for (const asset of remote)
      await verifyRemoteAsset(
        api,
        asset,
        assets.find((expected) => expected.name === asset.name)!,
      );
    for (const expected of assets)
      if (!remote.some((asset) => asset.name === expected.name)) {
        if (!upload) throw new Error("Missing release asset");
        await api.upload(release.upload_url, expected.name, expected.bytes);
      }
  };
  await reconcile(true);
  await reconcile(false);
  if (!enabled) return "draft";
  if (!immutableVerified)
    throw new Error("Repository release immutability must be verified before publication");
  await revalidate();
  const current = await releaseByTag(api.request, admitted.tag);
  if (current.id !== admitted.releaseId || current.target_commitish !== admitted.source)
    throw new Error("Release identity changed during upload");
  if (current.draft)
    await api.request(`/releases/${release.id}`, "PATCH", { draft: false, make_latest: "true" });
  // Lost publication responses are resolved by this same terminal read path on a later run.
  await verifyPublishedRelease(api, admitted, provenance.repository);
  return "published";
}
