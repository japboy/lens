import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { GitHub } from "./github.ts";
import { pages } from "./github.ts";
import type { AdmittedRelease } from "./admission.ts";
import { releaseByTag } from "./admission.ts";
import {
  parseReceipt,
  RECEIPT_NAME,
  verifyArtifactV2,
  type ArtifactManifestV2,
} from "./receipt.ts";
import {
  verifyBuildRun,
  verifyRemoteAsset,
  verifyPublishedRelease,
  type ProvenancePolicy,
  type RemoteAsset,
} from "./resume.ts";
import {
  BUNDLE_NAME,
  bundleAsset,
  bundleSigningAttempt,
  createBundle,
  readBundle,
  restoreBundle,
  MAX_BUNDLE_BYTES,
} from "./recovery-bundle.ts";
import { verifyBundleAttestation } from "./attestation.ts";
import { sha256 } from "./artifact.ts";

export type DurablePolicy = ProvenancePolicy & {
  verifyAttestation?: typeof verifyBundleAttestation;
  readmit: () => Promise<AdmittedRelease>;
};
// The API is explicit at every verification boundary; no mutable ambient client.
export async function verifyDurableArtifact(
  api: GitHub,
  directory: string,
  admitted: AdmittedRelease,
  policy: DurablePolicy,
) {
  const manifest = verifyArtifactV2(
    directory,
    { ...admitted, repository: policy.repository },
    true,
  );
  const bytes = readBundle(directory);
  const signingAttempt = bundleSigningAttempt(bytes, manifest.verificationAttempt);
  if (!createBundle(directory, manifest, signingAttempt).equals(bytes))
    throw new Error("Recovery bundle differs from artifact");
  (policy.verifyAttestation ?? verifyBundleAttestation)(join(directory, BUNDLE_NAME), {
    ...manifest,
    signingAttempt,
  });
  await verifyBuildRun(api.request, manifest, policy, signingAttempt);
  return { manifest, bytes };
}
export async function restoreDurableArtifact(
  api: GitHub,
  directory: string,
  admitted: AdmittedRelease,
  policy: DurablePolicy,
): Promise<void> {
  const remote = await pages<RemoteAsset>(api.request, `/releases/${admitted.releaseId}/assets`);
  const bundles = remote.filter((asset) => asset.name === BUNDLE_NAME);
  if (bundles.length !== 1 || bundles[0]!.size > MAX_BUNDLE_BYTES)
    throw new Error("Missing or invalid recovery bundle");
  const bytes = await api.download(`/releases/assets/${bundles[0]!.id}`);
  await verifyRemoteAsset(api, bundles[0]!, bundleAsset(bytes));
  const temporary = mkdtempSync(join(tmpdir(), "lens-durable-"));
  try {
    const extracted = join(temporary, "artifact");
    restoreBundle(bytes, extracted, { ...admitted, repository: policy.repository });
    writeFileSync(join(extracted, BUNDLE_NAME), bytes);
    await verifyDurableArtifact(api, extracted, admitted, policy);
    restoreBundle(bytes, directory, { ...admitted, repository: policy.repository });
    writeFileSync(join(directory, BUNDLE_NAME), bytes, { flag: "wx" });
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}
function durableReceipt(manifest: ArtifactManifestV2, admitted: AdmittedRelease, bytes: Buffer) {
  return parseReceipt(
    Buffer.from(
      JSON.stringify({
        schema: 3,
        signingAttempt: bundleSigningAttempt(bytes, manifest.verificationAttempt),
        bundle: bundleAsset(bytes),
        repository: manifest.repository,
        tag: manifest.tag,
        source: manifest.source,
        controller: manifest.controller,
        workflow: manifest.workflow,
        runId: manifest.runId,
        runAttempt: manifest.runAttempt,
        verificationAttempt: manifest.verificationAttempt,
        version: manifest.version,
        pullRequest: admitted.pullRequest,
        releaseId: admitted.releaseId,
        assets: manifest.assets,
      }),
    ),
    admitted,
    manifest.repository,
  );
}
async function readmit(admitted: AdmittedRelease, policy: DurablePolicy): Promise<void> {
  const current = await policy.readmit();
  for (const key of ["tag", "source", "version", "pullRequest", "releaseId", "legacy"] as const)
    if (current[key] !== admitted[key])
      throw new Error("Release admission changed during publication");
}
export async function publishDurableRelease(
  api: GitHub,
  directory: string,
  admitted: AdmittedRelease,
  policy: DurablePolicy,
  enabled: boolean,
  immutable: boolean,
): Promise<"draft" | "published" | "already-published"> {
  if (admitted.legacy) throw new Error("Legacy release cannot use durable publication");
  await readmit(admitted, policy);
  const release = await releaseByTag(api.request, admitted.tag);
  if (release.id !== admitted.releaseId || release.target_commitish !== admitted.source)
    throw new Error("Release identity changed");
  if (!release.draft) {
    await verifyPublishedRelease(api, admitted, policy.repository);
    return "already-published";
  }
  const { manifest, bytes } = await verifyDurableArtifact(api, directory, admitted, policy);
  const receiptBytes = Buffer.from(
    JSON.stringify(durableReceipt(manifest, admitted, bytes), null, 2) + "\n",
  );
  const assets = [
    { ...bundleAsset(bytes), bytes },
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
      remote.some((asset) => !assets.some((expected) => asset.name === expected.name))
    )
      throw new Error("Unexpected release assets");
    for (const asset of remote)
      await verifyRemoteAsset(
        api,
        asset,
        assets.find((expected) => expected.name === asset.name)!,
      );
    for (const asset of assets)
      if (!remote.some((existing) => existing.name === asset.name)) {
        if (!upload) throw new Error("Missing release asset");
        await api.upload(release.upload_url, asset.name, asset.bytes);
        // The recovery bundle must be fully readable before any distribution asset is uploaded.
        if (asset.name === BUNDLE_NAME) {
          const uploaded = (
            await pages<RemoteAsset>(api.request, `/releases/${release.id}/assets`)
          ).filter((item) => item.name === BUNDLE_NAME);
          if (uploaded.length !== 1) throw new Error("Recovery bundle upload not visible");
          await verifyRemoteAsset(api, uploaded[0]!, asset);
        }
      }
  };
  await reconcile(true);
  await reconcile(false);
  if (!enabled) return "draft";
  if (!immutable)
    throw new Error("Repository release immutability must be verified before publication");
  await readmit(admitted, policy);
  await api.request(`/releases/${release.id}`, "PATCH", { draft: false, make_latest: "true" });
  await verifyPublishedRelease(api, admitted, policy.repository);
  return "published";
}
