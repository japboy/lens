import { lstatSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { packageArtifact, sha256 } from "./artifact.ts";
import type { Asset, ReleaseManifest } from "./artifact.ts";
import type { AdmittedRelease } from "./admission.ts";
import { commitSha } from "./source.ts";
import { tagVersion } from "./version.ts";

export const RECEIPT_NAME = "release-receipt.json";
export const RELEASE_WORKFLOW = ".github/workflows/release-please.yml";
export type BuildIdentity = {
  repository: string;
  tag: string;
  source: string;
  controller: string;
  runId: string;
  runAttempt: string;
  workflow: typeof RELEASE_WORKFLOW;
};
export type ArtifactManifestV2 = Omit<ReleaseManifest, "schema"> & BuildIdentity & { schema: 2 };
export type ReleaseReceipt = BuildIdentity & {
  schema: 2;
  version: string;
  pullRequest: number;
  releaseId: number;
  artifactId: string;
  assets: Asset[];
};
export const positiveId = (value: unknown): value is string =>
  typeof value === "string" && /^[1-9]\d*$/u.test(value);
export function requireBuildIdentity(input: BuildIdentity): void {
  commitSha(input.source);
  commitSha(input.controller);
  tagVersion(input.tag);
  if (
    !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u.test(input.repository) ||
    !positiveId(input.runId) ||
    !positiveId(input.runAttempt) ||
    input.workflow !== RELEASE_WORKFLOW
  )
    throw new Error("Invalid build identity");
}
export function requireAssets(assets: Asset[], version: string): void {
  const names = [`Lens_${version}_aarch64.dmg`, "SHA256SUMS"];
  if (
    !Array.isArray(assets) ||
    assets.length !== names.length ||
    assets.some(
      (asset, i) =>
        !asset ||
        asset.name !== names[i] ||
        !Number.isSafeInteger(asset.size) ||
        asset.size < 1 ||
        typeof asset.sha256 !== "string" ||
        !/^[a-f0-9]{64}$/u.test(asset.sha256),
    )
  )
    throw new Error("Invalid distribution asset inventory");
}
export function packageArtifactV2(
  root: string,
  destination: string,
  input: BuildIdentity & { previousTag: string | null },
): ArtifactManifestV2 {
  requireBuildIdentity(input);
  const original = packageArtifact(root, destination, input);
  const manifest: ArtifactManifestV2 = {
    ...original,
    schema: 2,
    controller: input.controller,
    workflow: input.workflow,
  };
  writeFileSync(
    join(destination, "release-manifest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  return verifyArtifactV2(destination, input);
}
export function verifyArtifactV2(
  directory: string,
  expected: Pick<BuildIdentity, "tag" | "source" | "repository">,
): ArtifactManifestV2 {
  const manifest = JSON.parse(
    readFileSync(join(directory, "release-manifest.json"), "utf8"),
  ) as ArtifactManifestV2;
  requireBuildIdentity(manifest);
  const version = tagVersion(expected.tag);
  if (
    manifest.schema !== 2 ||
    manifest.tag !== expected.tag ||
    manifest.source !== expected.source ||
    manifest.repository !== expected.repository ||
    manifest.version !== version ||
    manifest.applicationSignature !== "adhoc" ||
    manifest.dmgSignature !== "unsigned" ||
    manifest.notarization !== "not-performed"
  )
    throw new Error("Release artifact provenance mismatch");
  requireAssets(manifest.assets, version);
  const files = [
    ...manifest.assets.map((asset) => asset.name),
    "release-manifest.json",
    "release-notes.md",
  ].sort();
  if (
    JSON.stringify(readdirSync(directory).sort()) !== JSON.stringify(files) ||
    files.some((name) => !lstatSync(join(directory, name)).isFile())
  )
    throw new Error("Release artifact must contain exactly four regular files");
  for (const asset of manifest.assets) {
    const bytes = readFileSync(join(directory, asset.name));
    if (asset.size !== bytes.length || asset.sha256 !== sha256(bytes))
      throw new Error("Release artifact digest conflict");
  }
  if (
    manifest.notesSha256 !== sha256(readFileSync(join(directory, "release-notes.md"))) ||
    readFileSync(join(directory, "SHA256SUMS"), "utf8") !==
      `${manifest.assets[0]!.sha256}  ${manifest.assets[0]!.name}\n`
  )
    throw new Error("Release artifact notes/checksum conflict");
  return manifest;
}
export function parseReceipt(
  bytes: Buffer,
  admitted: AdmittedRelease,
  repository: string,
): ReleaseReceipt {
  if (bytes.length > 64 * 1024) throw new Error("Release receipt exceeds size limit");
  const receipt = JSON.parse(bytes.toString("utf8")) as ReleaseReceipt;
  requireBuildIdentity(receipt);
  if (
    receipt.schema !== 2 ||
    receipt.tag !== admitted.tag ||
    receipt.source !== admitted.source ||
    receipt.repository !== repository ||
    receipt.version !== admitted.version ||
    receipt.pullRequest !== admitted.pullRequest ||
    receipt.releaseId !== admitted.releaseId ||
    !positiveId(receipt.artifactId)
  )
    throw new Error("Release receipt identity mismatch");
  requireAssets(receipt.assets, admitted.version);
  return receipt;
}
export function createReceipt(
  manifest: ArtifactManifestV2,
  admitted: AdmittedRelease,
  artifactId: string,
): ReleaseReceipt {
  const receipt: ReleaseReceipt = {
    schema: 2,
    repository: manifest.repository,
    tag: manifest.tag,
    source: manifest.source,
    controller: manifest.controller,
    workflow: manifest.workflow,
    runId: manifest.runId,
    runAttempt: manifest.runAttempt,
    version: manifest.version,
    pullRequest: admitted.pullRequest,
    releaseId: admitted.releaseId,
    artifactId,
    assets: manifest.assets,
  };
  return parseReceipt(Buffer.from(JSON.stringify(receipt)), admitted, manifest.repository);
}
