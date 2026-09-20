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
export type ArtifactManifestV2 = Omit<ReleaseManifest, "schema"> &
  BuildIdentity & { schema: 2; verificationAttempt: string };
export type ReleaseReceipt = BuildIdentity & {
  verificationAttempt: string;
  version: string;
  pullRequest: number;
  releaseId: number;
  assets: Asset[];
} & (
    | { schema: 2; artifactId: string; bundle?: never }
    | { schema: 3; bundle: Asset; signingAttempt: string; artifactId?: never }
  );
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
export function requireVerificationAttempt(
  identity: BuildIdentity & { verificationAttempt: string },
): void {
  requireBuildIdentity(identity);
  if (
    !positiveId(identity.verificationAttempt) ||
    BigInt(identity.verificationAttempt) < BigInt(identity.runAttempt)
  )
    throw new Error("Verification attempt must identify this build or a later retry");
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
    verificationAttempt: input.runAttempt,
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
  allowRecoveryBundle = false,
): ArtifactManifestV2 {
  const manifest = JSON.parse(
    readFileSync(join(directory, "release-manifest.json"), "utf8"),
  ) as ArtifactManifestV2;
  requireVerificationAttempt(manifest);
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
  const entries = readdirSync(directory);
  if (allowRecoveryBundle && entries.includes("release-recovery.json")) {
    const bundle = lstatSync(join(directory, "release-recovery.json"));
    if (!bundle.isFile() || bundle.size < 1 || bundle.size > 256 * 1024 * 1024)
      throw new Error("Invalid recovery bundle file");
    entries.splice(entries.indexOf("release-recovery.json"), 1);
  }
  if (
    JSON.stringify(entries.sort()) !== JSON.stringify(files) ||
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
  requireVerificationAttempt(receipt);
  if (
    ![2, 3].includes(receipt.schema) ||
    receipt.tag !== admitted.tag ||
    receipt.source !== admitted.source ||
    receipt.repository !== repository ||
    receipt.version !== admitted.version ||
    receipt.pullRequest !== admitted.pullRequest ||
    receipt.releaseId !== admitted.releaseId ||
    (receipt.schema === 2 && !positiveId(receipt.artifactId))
  )
    throw new Error("Release receipt identity mismatch");
  if (
    receipt.schema === 3 &&
    (!positiveId(receipt.signingAttempt) ||
      BigInt(receipt.signingAttempt) < BigInt(receipt.verificationAttempt) ||
      !receipt.bundle ||
      receipt.bundle.name !== "release-recovery.json" ||
      !Number.isSafeInteger(receipt.bundle.size) ||
      receipt.bundle.size < 1 ||
      receipt.bundle.size > 256 * 1024 * 1024 ||
      !/^[a-f0-9]{64}$/u.test(receipt.bundle.sha256) ||
      receipt.artifactId !== undefined)
  )
    throw new Error("Invalid durable receipt bundle");
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
    verificationAttempt: manifest.verificationAttempt,
    version: manifest.version,
    pullRequest: admitted.pullRequest,
    releaseId: admitted.releaseId,
    artifactId,
    assets: manifest.assets,
  };
  return parseReceipt(Buffer.from(JSON.stringify(receipt)), admitted, manifest.repository);
}

/** Promotion runs in the trusted controller after the aggregate gate succeeds.
 * Preserve the original build attempt when only failed jobs were rerun. */
export function promoteArtifactV2(
  directory: string,
  expected: Pick<BuildIdentity, "repository" | "tag" | "source" | "controller" | "runId">,
  verificationAttempt: string,
): ArtifactManifestV2 {
  const manifest = verifyArtifactV2(directory, expected);
  if (manifest.controller !== expected.controller || manifest.runId !== expected.runId)
    throw new Error("Candidate promotion controller/run identity mismatch");
  const promoted = { ...manifest, verificationAttempt };
  requireVerificationAttempt(promoted);
  writeFileSync(join(directory, "release-manifest.json"), `${JSON.stringify(promoted, null, 2)}\n`);
  return promoted;
}
