import { lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { sha256, type Asset } from "./artifact.ts";
import { verifyArtifactV2, type ArtifactManifestV2 } from "./receipt.ts";

export const BUNDLE_NAME = "release-recovery.json";
export const MAX_BUNDLE_BYTES = 256 * 1024 * 1024;
type Bundle = { schema: 1; signingAttempt: string; files: { name: string; content: string }[] };
const names = (version: string) => [
  `Lens_${version}_aarch64.dmg`,
  "SHA256SUMS",
  "release-manifest.json",
  "release-notes.md",
];

export function createBundle(
  directory: string,
  manifest: ArtifactManifestV2,
  signingAttempt = manifest.verificationAttempt,
): Buffer {
  requireSigningAttempt(signingAttempt, manifest.verificationAttempt);
  const inputSize = names(manifest.version).reduce(
    (sum, name) => sum + lstatSync(join(directory, name)).size,
    0,
  );
  if (inputSize > MAX_BUNDLE_BYTES * 0.7) throw new Error("Recovery bundle exceeds size limit");
  verifyArtifactV2(directory, manifest, true);
  const bundle: Bundle = {
    schema: 1,
    signingAttempt,
    files: names(manifest.version).map((name) => ({
      name,
      content: readFileSync(join(directory, name)).toString("base64"),
    })),
  };
  const bytes = Buffer.from(JSON.stringify(bundle) + "\n");
  if (bytes.length > MAX_BUNDLE_BYTES) throw new Error("Recovery bundle exceeds size limit");
  return bytes;
}
function requireSigningAttempt(attempt: unknown, gate: string): asserts attempt is string {
  if (typeof attempt !== "string" || !/^[1-9]\d*$/u.test(attempt) || BigInt(attempt) < BigInt(gate))
    throw new Error("Invalid bundle signing attempt");
}
export function bundleSigningAttempt(bytes: Buffer, gate: string): string {
  const value = JSON.parse(bytes.toString("utf8")) as Bundle;
  requireSigningAttempt(value.signingAttempt, gate);
  return value.signingAttempt;
}
export function bundleAsset(bytes: Buffer): Asset {
  if (!bytes.length || bytes.length > MAX_BUNDLE_BYTES)
    throw new Error("Invalid recovery bundle size");
  return { name: BUNDLE_NAME, size: bytes.length, sha256: sha256(bytes) };
}
export function restoreBundle(
  bytes: Buffer,
  directory: string,
  expected: {
    tag: string;
    source: string;
    repository: string;
    version: string;
  },
): ArtifactManifestV2 {
  bundleAsset(bytes);
  const value = JSON.parse(bytes.toString("utf8")) as Bundle;
  const required = names(expected.version);
  if (
    value.schema !== 1 ||
    !Array.isArray(value.files) ||
    JSON.stringify(value.files.map((file) => file.name)) !== JSON.stringify(required) ||
    value.files.some((file) => typeof file.content !== "string")
  )
    throw new Error("Invalid recovery bundle inventory");
  const decoded = value.files.map((file) => {
    const content = Buffer.from(file.content, "base64");
    if (!content.length || content.toString("base64") !== file.content)
      throw new Error("Invalid recovery bundle encoding");
    return { name: file.name, content };
  });
  mkdirSync(directory, { recursive: true });
  if (readdirSync(directory).length) throw new Error("Recovery destination must be empty");
  for (const file of decoded)
    writeFileSync(join(directory, file.name), file.content, { flag: "wx" });
  const manifest = verifyArtifactV2(directory, expected);
  if (
    !createBundle(
      directory,
      manifest,
      bundleSigningAttempt(bytes, manifest.verificationAttempt),
    ).equals(bytes)
  )
    throw new Error("Recovery bundle is not canonical");
  return manifest;
}
export function readBundle(directory: string): Buffer {
  const path = join(directory, BUNDLE_NAME);
  if (!lstatSync(path).isFile()) throw new Error("Recovery bundle must be a regular file");
  const bytes = readFileSync(path);
  bundleAsset(bytes);
  return bytes;
}
