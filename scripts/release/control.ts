import type { Request } from "./github.ts";
import { optional, pages } from "./github.ts";
import { annotation, changelogSection, commitSha, git, readSource, requireMain } from "./source.ts";
import type { TagAnnotation } from "./source.ts";
import { compareVersions, tagVersion } from "./version.ts";

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

export function validateReleasePr(pr: PullRequest, sha: string, repository: string): void {
  if (
    !pr.merged ||
    pr.state !== "closed" ||
    pr.merge_commit_sha !== commitSha(sha) ||
    pr.base.ref !== "main" ||
    pr.base.repo.full_name !== repository ||
    pr.head.repo?.full_name !== repository ||
    pr.head.ref !== RELEASE_BRANCH ||
    !pr.labels.some((label) => [PENDING, TAGGED].includes(label.name))
  )
    throw new Error("Commit is not an admitted merged release PR");
}

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
    if (!name.startsWith("v") || name === current) continue;
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

export async function ensureTag(request: Request, data: TagAnnotation): Promise<void> {
  const tag = `v${data.version}`;
  const message = annotation(data);
  const check = async (ref: { object: { type: string; sha: string } }) => {
    if (ref.object.type !== "tag") throw new Error("Existing release tag is lightweight");
    const existing = await request<{
      tag: string;
      message: string;
      object: { type: string; sha: string };
    }>(`/git/tags/${ref.object.sha}`);
    if (
      existing.tag !== tag ||
      existing.message.trimEnd() !== message ||
      existing.object.type !== "commit" ||
      existing.object.sha !== data.commit
    )
      throw new Error("Existing release tag conflicts with the admitted source");
  };
  const refPath = `/git/ref/tags/${tag}`;
  const existing = await optional(() =>
    request<{ object: { type: string; sha: string } }>(refPath),
  );
  if (existing) return check(existing);
  const object = await request<{ sha: string }>("/git/tags", "POST", {
    tag,
    message,
    object: data.commit,
    type: "commit",
  });
  // A lost response is recovered by reading the exact reference on the next run.
  await request("/git/refs", "POST", { ref: `refs/tags/${tag}`, sha: object.sha });
  await check(await request(refPath));
}

export async function control(
  request: Request,
  root: string,
  eventSha: string,
  repository: string,
): Promise<"tagged" | "update-pr" | "awaiting-publication"> {
  requireMain(root, eventSha);
  const associated = await pages<PullRequest>(request, `/commits/${eventSha}/pulls`);
  const pending = await pages<{ number: number }>(
    request,
    `/issues?state=closed&labels=${encodeURIComponent(PENDING)}`,
  );
  const candidates = new Map<number, PullRequest>();
  for (const item of [...associated, ...pending]) {
    const pr = await request<PullRequest>(`/pulls/${item.number}`);
    if (
      pr.merged &&
      pr.head.ref === RELEASE_BRANCH &&
      pr.labels.some((label) => label.name === PENDING)
    )
      candidates.set(pr.number, pr);
  }
  if (candidates.size > 1)
    throw new Error("Multiple unprocessed release PRs require reconciliation");
  const pr = [...candidates.values()][0];
  if (pr) {
    const sha = commitSha(pr.merge_commit_sha ?? "");
    validateReleasePr(pr, sha, repository);
    requireMain(root, sha);
    const state = readSource(root, sha);
    if (!state.bootstrapped) throw new Error("Release manifest is missing");
    changelogSection(git(root, "show", `${sha}:CHANGELOG.md`), state.version);
    await previousRelease(request, state.version);
    await ensureTag(request, {
      schema: 1,
      version: state.version,
      commit: sha,
      pullRequest: pr.number,
    });
    await request(`/issues/${pr.number}/labels`, "POST", { labels: [TAGGED] });
    await request(`/issues/${pr.number}/labels/${encodeURIComponent(PENDING)}`, "DELETE");
    return "tagged";
  }
  const source = readSource(root, eventSha);
  if (!source.bootstrapped) {
    if ((await pages(request, "/tags")).length || (await pages(request, "/releases")).length)
      throw new Error("An empty manifest cannot reuse a repository with release state");
    return "update-pr";
  }
  const published = await optional(() => request<Release>(`/releases/tags/v${source.version}`));
  if (!published || published.draft) return "awaiting-publication";
  await previousRelease(request, source.version);
  return "update-pr";
}
