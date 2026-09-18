import type { Request } from "./github.ts";
import { pages } from "./github.ts";
import type { PullRequest, Release } from "./control.ts";
import { changelogSection, commitSha, git, inspectTag, readSource, requireMain } from "./source.ts";
import { tagVersion } from "./version.ts";

export const OWNED_RELEASE_BRANCHES = [
  "release-please--branches--main",
  "release-please--branches--main--components--lens",
] as const;
export type AdmittedRelease = {
  tag: string;
  version: string;
  source: string;
  pullRequest: number;
  releaseId: number;
  legacy: boolean;
  draft: boolean;
};

// Labels are diagnostic only: failed label updates must not redefine source identity.
export function requireMergedReleasePr(pr: PullRequest, source: string, repository: string): void {
  if (
    !Number.isSafeInteger(pr.number) ||
    pr.number < 1 ||
    !pr.merged ||
    pr.state !== "closed" ||
    pr.merge_commit_sha !== commitSha(source) ||
    pr.base.ref !== "main" ||
    pr.base.repo.full_name !== repository ||
    pr.head.repo?.full_name !== repository ||
    !OWNED_RELEASE_BRANCHES.some((branch) => branch === pr.head.ref)
  )
    throw new Error("Source is not an owned merged release PR");
}

export async function releaseByTag(request: Request, tag: string): Promise<Release> {
  tagVersion(tag);
  const matches = (await pages<Release>(request, "/releases")).filter((r) => r.tag_name === tag);
  if (matches.length !== 1) throw new Error("Expected one existing Release Please release");
  const release = matches[0]!;
  if (!Number.isSafeInteger(release.id) || release.id < 1 || release.prerelease)
    throw new Error("Invalid stable release identity");
  return release;
}

export async function admitRelease(
  request: Request,
  root: string,
  repository: string,
  tag: string,
): Promise<AdmittedRelease> {
  const version = tagVersion(tag);
  const ref = `refs/tags/${tag}`;
  const kind = git(root, "cat-file", "-t", ref);
  if (kind !== "commit" && kind !== "tag") throw new Error("Release tag must resolve to a commit");
  const legacy = kind === "tag";
  const annotation = legacy ? inspectTag(root, tag) : undefined;
  const source = commitSha(git(root, "rev-parse", `${ref}^{commit}`));
  requireMain(root, source);
  const state = readSource(root, source);
  if (!state.bootstrapped || state.version !== version)
    throw new Error("Release source/version mismatch");
  changelogSection(git(root, "show", `${source}:CHANGELOG.md`), version);
  const candidates = await pages<Pick<PullRequest, "number">>(request, `/commits/${source}/pulls`);
  const prs: PullRequest[] = [];
  for (const item of candidates) {
    const pr = await request<PullRequest>(`/pulls/${item.number}`);
    if (
      pr.merge_commit_sha === source &&
      OWNED_RELEASE_BRANCHES.some((branch) => branch === pr.head.ref)
    ) {
      requireMergedReleasePr(pr, source, repository);
      prs.push(pr);
    }
  }
  if (prs.length !== 1 || (annotation && annotation.pullRequest !== prs[0]!.number))
    throw new Error("Expected one source-bound merged release PR");
  const remote = await request<{ object: { type: string; sha: string } }>(`/git/ref/tags/${tag}`);
  if (remote.object.type !== kind || remote.object.sha !== git(root, "rev-parse", ref))
    throw new Error("Remote release tag/source conflict");
  let remoteSource = remote.object.sha;
  if (remote.object.type === "tag") {
    const object = await request<{ object: { type: string; sha: string } }>(
      `/git/tags/${remoteSource}`,
    );
    if (object.object.type !== "commit") throw new Error("Nested release tags are not admitted");
    remoteSource = object.object.sha;
  } else if (remote.object.type !== "commit") throw new Error("Invalid remote release ref");
  if (remoteSource !== source) throw new Error("Remote release tag/source conflict");
  const release = await releaseByTag(request, tag);
  if (release.target_commitish !== source) throw new Error("Release target/source conflict");
  return {
    tag,
    version,
    source,
    pullRequest: prs[0]!.number,
    releaseId: release.id,
    legacy,
    draft: release.draft,
  };
}
