import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { commitSha } from "./source.ts";
import { stableVersion, tagVersion } from "./version.ts";

export const sha256 = (bytes: Buffer | string): string =>
  createHash("sha256").update(bytes).digest("hex");
export type Asset = { name: string; size: number; sha256: string };
export type ReleaseManifest = {
  schema: 1;
  tag: string;
  source: string;
  repository: string;
  runId: string;
  runAttempt: string;
  version: string;
  previousTag: string | null;
  assets: Asset[];
  notesSha256: string;
  configuration: Record<string, string>;
  tools: Record<string, string>;
  applicationSignature: "adhoc";
  dmgSignature: "unsigned";
  notarization: "not-performed";
};
export const CONFIGURATION_FILES = [
  "mise.lock",
  "pnpm-lock.yaml",
  "Cargo.lock",
  "apps/desktop/src-tauri/tauri.conf.json",
  "apps/desktop/src-tauri/tauri.macos.conf.json",
  "apps/desktop/src-tauri/tauri.release.conf.json",
];

function installationText(assetName: string): string {
  return `## Install and update

Lens is in early development. Packaged builds are publicly available on GitHub Releases.
Requires **Apple Silicon and macOS 15.2 or later**. Download **${assetName}** and **SHA256SUMS**.
The application has an ad-hoc signature. The DMG is unsigned; neither is Apple-notarized.

Verify both downloaded files in the same directory:

\`\`\`sh
shasum -a 256 -c SHA256SUMS
\`\`\`

Open the DMG, drag Lens.app to Applications, eject the image, and launch the installed copy.
For Gatekeeper, attempt launch and use System Settings > Privacy & Security > Open Anyway
only after verifying the download. Follow [Apple's first-launch instructions](https://support.apple.com/en-us/102445).
For updates, quit Lens before replacing the app. Accessibility and Screen Recording
permissions may need to be granted again. There is no automatic updater.
Agent runtimes are downloaded separately; select and authenticate an Agent in Settings.`;
}

export function installationNotes(version: string): string {
  return installationText(`Lens_${stableVersion(version)}_aarch64.dmg`);
}

export function releaseNotes(
  section: string,
  manifest: Pick<ReleaseManifest, "tag" | "source" | "repository" | "runId" | "previousTag">,
  asset: Asset,
): string {
  const { repository, tag, source, runId, previousTag } = manifest;
  const base = `https://github.com/${repository}`;
  return `${section}

${installationText(asset.name)}

## Provenance

- Tag: ${tag}
- Commit: [${source}](${base}/commit/${source})
- Original build: [workflow run ${runId}](${base}/actions/runs/${runId})
- DMG SHA-256: \`${asset.sha256}\`
${previousTag ? `- Changes: [${previousTag}...${tag}](${base}/compare/${previousTag}...${tag})\n` : ""}
`;
}

export function verifyArtifact(
  directory: string,
  expected: { tag: string; source: string; repository: string; runId: string },
): ReleaseManifest {
  const manifest: ReleaseManifest = JSON.parse(
    readFileSync(join(directory, "release-manifest.json"), "utf8"),
  );
  const version = tagVersion(expected.tag);
  commitSha(expected.source);
  const names = [`Lens_${version}_aarch64.dmg`, "SHA256SUMS"];
  if (
    manifest.schema !== 1 ||
    manifest.tag !== expected.tag ||
    manifest.source !== expected.source ||
    manifest.repository !== expected.repository ||
    manifest.runId !== expected.runId ||
    manifest.version !== version ||
    !/^[1-9]\d*$/u.test(manifest.runId) ||
    !/^[1-9]\d*$/u.test(manifest.runAttempt) ||
    manifest.applicationSignature !== "adhoc" ||
    manifest.dmgSignature !== "unsigned" ||
    manifest.notarization !== "not-performed" ||
    JSON.stringify(manifest.assets.map((asset) => asset.name)) !== JSON.stringify(names)
  )
    throw new Error("Release artifact provenance or asset contract mismatch");
  const files = [...names, "release-manifest.json", "release-notes.md"].sort();
  if (
    JSON.stringify(readdirSync(directory).sort()) !== JSON.stringify(files) ||
    files.some((name) => !lstatSync(join(directory, name)).isFile())
  )
    throw new Error("Release artifact must contain exactly the four regular files");
  for (const asset of manifest.assets) {
    const bytes = readFileSync(join(directory, asset.name));
    if (asset.size !== bytes.length || asset.sha256 !== sha256(bytes))
      throw new Error(`Release artifact size/digest mismatch: ${asset.name}`);
  }
  const notes = readFileSync(join(directory, "release-notes.md"));
  if (
    manifest.notesSha256 !== sha256(notes) ||
    readFileSync(join(directory, "SHA256SUMS"), "utf8") !==
      `${manifest.assets[0]!.sha256}  ${names[0]}\n`
  )
    throw new Error("Release notes or checksum file mismatch");
  return manifest;
}
