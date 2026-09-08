import { createHash } from "node:crypto";
import {
  copyFileSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { changelogSection, cleanSource, commitSha, git } from "./source.ts";
import { readVersion, tagVersion } from "./version.ts";

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
  "scripts/workspace-variants.json",
  "apps/desktop/src-tauri/tauri.conf.json",
  "apps/desktop/src-tauri/tauri.macos.conf.json",
  "apps/desktop/src-tauri/tauri.release.conf.json",
  "apps/desktop/src-tauri/tauri.output.conf.json",
];

export function releaseNotes(
  section: string,
  manifest: Pick<ReleaseManifest, "tag" | "source" | "repository" | "runId" | "previousTag">,
  asset: Asset,
): string {
  const { repository, tag, source, runId, previousTag } = manifest;
  const base = `https://github.com/${repository}`;
  return `${section}

## Install and update

This initial distribution is for testers with read access to this private repository.
Requires **Apple Silicon and macOS 15.2 or later**. Download **${asset.name}** and **SHA256SUMS**.
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
Agent runtimes are downloaded separately; select and authenticate an Agent in Settings.

## Provenance

- Tag: ${tag}
- Commit: [${source}](${base}/commit/${source})
- Original build: [workflow run ${runId}](${base}/actions/runs/${runId})
- DMG SHA-256: \`${asset.sha256}\`
${previousTag ? `- Changes: [${previousTag}...${tag}](${base}/compare/${previousTag}...${tag})\n` : ""}
`;
}

export function packageArtifact(
  root: string,
  destination: string,
  input: {
    tag: string;
    source: string;
    repository: string;
    runId: string;
    runAttempt: string;
    previousTag: string | null;
  },
): ReleaseManifest {
  cleanSource(root, input.source);
  const version = tagVersion(input.tag);
  const state = readVersion(root);
  if (!state.bootstrapped || state.version !== version)
    throw new Error("Artifact source/version mismatch");
  const name = `Lens_${version}_aarch64.dmg`;
  const source = join(root, "target/aarch64-apple-darwin/release/bundle/dmg", name);
  const bytes = readFileSync(source);
  const dmg: Asset = { name, size: bytes.length, sha256: sha256(bytes) };
  const sums = Buffer.from(`${dmg.sha256}  ${name}\n`);
  const section = changelogSection(git(root, "show", `${input.source}:CHANGELOG.md`), version);
  const notes = releaseNotes(section, input, dmg);
  const command = (file: string, args: string[]) =>
    execFileSync(file, args, { cwd: root, encoding: "utf8" }).trim();
  const manifest: ReleaseManifest = {
    schema: 1,
    ...input,
    version,
    assets: [dmg, { name: "SHA256SUMS", size: sums.length, sha256: sha256(sums) }],
    notesSha256: sha256(notes),
    configuration: Object.fromEntries(
      CONFIGURATION_FILES.map((path) => [path, sha256(readFileSync(join(root, path)))]),
    ),
    tools: {
      node: process.version,
      pnpm: command("pnpm", ["--version"]),
      rustc: command("rustc", ["-vV"]),
      tauri: command("pnpm", ["--dir", "apps/desktop", "exec", "tauri", "--version"]),
      xcode: command("xcodebuild", ["-version"]),
      sdk: command("xcrun", ["--show-sdk-version"]),
      os: command("sw_vers", []),
      runnerImage: process.env.ImageVersion ?? "local",
    },
    applicationSignature: "adhoc",
    dmgSignature: "unsigned",
    notarization: "not-performed",
  };
  mkdirSync(destination, { recursive: true });
  if (readdirSync(destination).length)
    throw new Error("Release artifact destination must be empty");
  copyFileSync(source, join(destination, name));
  writeFileSync(join(destination, "SHA256SUMS"), sums);
  writeFileSync(join(destination, "release-notes.md"), notes);
  writeFileSync(
    join(destination, "release-manifest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  verifyArtifact(destination, input);
  return manifest;
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
