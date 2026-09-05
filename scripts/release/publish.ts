import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { GitHub } from "./github.ts";
import { optional, pages } from "./github.ts";
import type { Release } from "./control.ts";
import { sha256, verifyArtifact } from "./artifact.ts";
import type { Asset } from "./artifact.ts";

type RemoteAsset = { id: number; name: string; size: number; state: string; digest: string | null };
export function requireReleaseJobs(portable: string, common: string, native: string): void {
  const expected = "success";
  if ([portable, common, native].some((result) => result !== expected))
    throw new Error("Incomplete release verification jobs");
}

export async function publish(
  api: GitHub,
  directory: string,
  expected: {
    tag: string;
    source: string;
    repository: string;
    runId: string;
  },
  enabled: boolean,
  immutableVerified: boolean,
): Promise<"draft" | "published" | "already-published"> {
  const manifest = verifyArtifact(directory, expected);
  const body = readFileSync(join(directory, "release-notes.md"), "utf8");
  let release = await optional(() => api.request<Release>(`/releases/tags/${expected.tag}`));
  // Drafts are not guaranteed to appear at the tag endpoint; enumerate with writer auth.
  if (!release) {
    const matches = (await pages<Release>(api.request, "/releases")).filter(
      (item) => item.tag_name === expected.tag,
    );
    if (matches.length > 1) throw new Error("Duplicate releases for tag");
    release = matches[0];
  }
  if (!release)
    release = await api.request<Release>("/releases", "POST", {
      tag_name: expected.tag,
      target_commitish: expected.source,
      name: `Lens ${manifest.version}`,
      body,
      draft: true,
      prerelease: false,
    });
  if (
    release.tag_name !== expected.tag ||
    release.target_commitish !== expected.source ||
    release.prerelease ||
    release.name !== `Lens ${manifest.version}` ||
    release.body !== body
  )
    throw new Error("Existing release metadata conflicts with the verified artifact");
  const selected = release;
  const checkAsset = async (remote: RemoteAsset, asset: Asset) => {
    if (remote.state !== "uploaded" || remote.size !== asset.size)
      throw new Error(`Existing release asset state/size conflict: ${asset.name}`);
    // Download even when the API digest is present, so the uploaded bytes are verified.
    if (
      (remote.digest !== null &&
        remote.digest !== undefined &&
        remote.digest !== `sha256:${asset.sha256}`) ||
      sha256(await api.download(`/releases/assets/${remote.id}`)) !== asset.sha256
    )
      throw new Error(`Existing release asset digest conflict: ${asset.name}`);
  };
  const reconcile = async (allowUpload: boolean) => {
    const remote = await pages<RemoteAsset>(api.request, `/releases/${selected.id}/assets`);
    if (
      new Set(remote.map((asset) => asset.name)).size !== remote.length ||
      remote.some(
        (asset) => !manifest.assets.some((expectedAsset) => expectedAsset.name === asset.name),
      )
    )
      throw new Error("Unexpected release assets");
    for (const asset of manifest.assets) {
      const existing = remote.find((entry) => entry.name === asset.name);
      if (existing) await checkAsset(existing, asset);
      else if (allowUpload)
        await api.upload(
          selected.upload_url,
          asset.name,
          readFileSync(join(directory, asset.name)),
        );
      else throw new Error("Release asset is missing");
    }
  };
  if (!release.draft) {
    await reconcile(false);
    return "already-published";
  }
  await reconcile(true);
  await reconcile(false);
  if (!enabled) return "draft";
  if (!immutableVerified)
    throw new Error("Enable repository release immutability before publication");
  // If this response is lost, a rerun verifies the same terminal release above.
  release = await api.request<Release>(`/releases/${release.id}`, "PATCH", {
    draft: false,
    make_latest: "true",
  });
  if (release.draft || release.tag_name !== expected.tag)
    throw new Error("Publication did not reach terminal state");
  await reconcile(false);
  return "published";
}
