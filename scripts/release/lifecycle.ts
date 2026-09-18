import type { Request } from "./github.ts";
import { pages } from "./github.ts";
import type { PullRequest, Release } from "./control.ts";
import { PENDING, previousRelease } from "./control.ts";
import { changelogSection, commitSha, git, readSource, requireMain } from "./source.ts";
import { tagVersion } from "./version.ts";

export const PROPOSAL_BRANCH = "release-please--branches--main--components--lens";
export const LEGACY_BRANCH = "release-please--branches--main";
export type LifecyclePlan = { state: "proposal" } | { state: "release"; tag: string };
type Operations = { createReleases(): Promise<unknown>; propose(): Promise<unknown> };

export async function requireStrictChecks(request: Request): Promise<void> {
  const rules = await request<
    {
      type: string;
      ruleset_id: number;
      parameters?: {
        strict_required_status_checks_policy?: boolean;
        required_status_checks?: { context: string }[];
      };
    }[]
  >("/rules/branches/main");
  const strict = rules.filter(
    (rule) =>
      rule.type === "required_status_checks" &&
      rule.parameters?.strict_required_status_checks_policy === true &&
      rule.parameters.required_status_checks?.some((check) => check.context === "code-quality"),
  );
  for (const rule of strict) {
    if (!Number.isSafeInteger(rule.ruleset_id) || rule.ruleset_id < 1) continue;
    const policy = await request<{ enforcement: string; bypass_actors: unknown[] }>(
      `/rulesets/${rule.ruleset_id}`,
    );
    if (
      policy.enforcement === "active" &&
      Array.isArray(policy.bypass_actors) &&
      policy.bypass_actors.length === 0
    )
      return;
  }
  throw new Error(
    "Release mutations require strict up-to-date code-quality checks without bypass actors on main",
  );
}

export async function lifecycle(
  request: Request,
  root: string,
  repository: string,
  controllerSha: string,
  operations: Operations,
  selectedTag = "",
): Promise<LifecyclePlan> {
  requireMain(root, commitSha(controllerSha));
  const pulls = await pages<PullRequest>(request, "/pulls?state=all&base=main");
  const candidates = pulls.filter(
    (pr) =>
      pr.head.repo?.full_name === repository &&
      pr.base.repo.full_name === repository &&
      [PROPOSAL_BRANCH, LEGACY_BRANCH].includes(pr.head.ref),
  );
  const owned = await Promise.all(
    candidates
      .filter((pr) => pr.state === "open" || pr.labels.some((label) => label.name === PENDING))
      .map((pr) => request<PullRequest>(`/pulls/${pr.number}`)),
  );
  const legacy = owned.filter(
    (pr) =>
      pr.head.ref === LEGACY_BRANCH &&
      (pr.state === "open" || (pr.merged && pr.labels.some((label) => label.name === PENDING))),
  );
  if (legacy.length) throw new Error("Complete old-format release PRs before migration");
  const pending = owned.filter(
    (pr) =>
      pr.head.ref === PROPOSAL_BRANCH &&
      pr.merged &&
      pr.labels.some((label) => label.name === PENDING),
  );
  if (pending.length > 1) throw new Error("Multiple pending merged release PRs");
  const releases = await pages<Release>(request, "/releases");
  const drafts = releases.filter((release) => release.draft);
  if (drafts.length > 1) throw new Error("Multiple unfinished releases");
  const pr = pending[0];
  let pendingTag: string | undefined;
  if (pr) {
    if (pr.state !== "closed" || pr.base.ref !== "main") throw new Error("Invalid release PR");
    const sha = commitSha(pr.merge_commit_sha ?? "");
    requireMain(root, sha);
    const source = readSource(root, sha);
    if (!source.bootstrapped) throw new Error("Missing release manifest");
    changelogSection(git(root, "show", `${sha}:CHANGELOG.md`), source.version);
    pendingTag = `v${source.version}`;
    await previousRelease(request, source.version);
  }
  const draftTag = drafts[0]?.tag_name;
  if (draftTag) tagVersion(draftTag);
  if (pendingTag && draftTag && pendingTag !== draftTag)
    throw new Error("Pending PR and draft disagree");
  if (selectedTag) {
    tagVersion(selectedTag);
    if ((pendingTag && pendingTag !== selectedTag) || (draftTag && draftTag !== selectedTag))
      throw new Error("Selected release conflicts with unfinished release");
  }
  const tag = selectedTag || pendingTag || draftTag;
  if (pendingTag) {
    await requireStrictChecks(request);
    try {
      await operations.createReleases();
    } catch (error) {
      // Upstream repairs pending labels before reporting a duplicate. No other
      // error is converted to success; preflight independently admits the source.
      if (!(error instanceof Error) || error.name !== "DuplicateReleaseError") throw error;
    }
    const observed = (await pages<Release>(request, "/releases")).filter((r) => r.tag_name === tag);
    if (observed.length !== 1 || observed[0]!.target_commitish !== pr!.merge_commit_sha)
      throw new Error("Standard release creation produced no unique matching release");
    const ref = await request<{ object: { type: string; sha: string } }>(`/git/ref/tags/${tag}`);
    if (ref.object.type !== "commit" || ref.object.sha !== pr!.merge_commit_sha)
      throw new Error("Standard release tag does not match merged source");
  }
  if (tag) return { state: "release", tag };
  // Reject orphan stable tags and unpublished manifest versions before proposing.
  const current = readSource(root, controllerSha);
  if (current.bootstrapped) {
    await previousRelease(request, current.version);
    const published = releases.filter(
      (r) => r.tag_name === `v${current.version}` && !r.draft && !r.prerelease,
    );
    if (published.length !== 1) throw new Error("Current version has no published release");
  } else if (releases.length || (await pages(request, "/tags")).length) {
    throw new Error("Initial release conflicts with existing release state");
  }
  await requireStrictChecks(request);
  await operations.propose();
  return { state: "proposal" };
}
