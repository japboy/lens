import type { Request } from "./github.ts";
import { pages } from "./github.ts";
import { compareVersions, STABLE_VERSION, tagVersion } from "./version.ts";

export const RELEASE_BRANCH = "release-please--branches--main";
export const PENDING = "autorelease: pending";
export const TAGGED = "autorelease: tagged";
export type PullRequest = {
  number: number;
  merged: boolean;
  merge_commit_sha: string | null;
  state: string;
  base: { ref: string; repo: { full_name: string } };
  head: { ref: string; repo: { full_name: string } | null };
  labels: { name: string }[];
};
export type Release = {
  id: number;
  tag_name: string;
  target_commitish: string;
  draft: boolean;
  prerelease: boolean;
  name: string;
  body: string;
  upload_url: string;
  immutable?: boolean;
};

// Every existing stable tag must already have a published release before a next version.
// The current tag is excluded only for an idempotent resume of that exact release.
export async function previousRelease(
  request: Request,
  version: string,
): Promise<string | undefined> {
  const tags = await pages<{ name: string }>(request, "/tags");
  const releases = await pages<Release>(request, "/releases");
  const current = `v${version}`;
  let previous: string | undefined;
  for (const { name } of tags) {
    if (!name.startsWith("v") || !STABLE_VERSION.test(name.slice(1)) || name === current) continue;
    const other = tagVersion(name);
    if (compareVersions(other, version) >= 0)
      throw new Error("Release versions must advance monotonically");
    const published = releases.filter(
      (entry) => entry.tag_name === name && !entry.draft && !entry.prerelease,
    );
    if (published.length !== 1) throw new Error(`Previous release is not published: ${name}`);
    if (!previous || compareVersions(other, tagVersion(previous)) > 0) previous = name;
  }
  for (const release of releases)
    if (
      release.tag_name !== current &&
      (release.draft || !tags.some((tag) => tag.name === release.tag_name))
    )
      throw new Error("An unresolved or untagged release already exists");
  if (!previous && version !== "0.1.0") throw new Error("First release must be 0.1.0");
  return previous;
}
