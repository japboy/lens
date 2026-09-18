import { describe, expect, it, vi } from "vitest";
import { lifecycle, requireStrictChecks, PROPOSAL_BRANCH, LEGACY_BRANCH } from "./lifecycle.ts";
import type { Request } from "./github.ts";
const sha = "a".repeat(40);
const source = vi.hoisted(() => ({ version: "0.4.1", bootstrapped: true }));
vi.mock("./source.ts", () => ({
  requireMain: vi.fn<() => void>(),
  commitSha: (value: string) => value,
  readSource: () => source,
  changelogSection: vi.fn<() => void>(),
  git: () => "changelog",
}));
const repo = "fixture/lens";
function fixture(
  options: {
    strict?: boolean;
    bypass?: boolean;
    legacy?: boolean;
    pending?: boolean;
    draft?: string;
    published?: boolean;
    orphan?: boolean;
  } = {},
) {
  const pr = {
    number: 94,
    merged: !!options.pending,
    state: options.pending ? "closed" : "open",
    merge_commit_sha: sha,
    base: { ref: "main", repo: { full_name: repo } },
    head: { ref: options.legacy ? LEGACY_BRANCH : PROPOSAL_BRANCH, repo: { full_name: repo } },
    labels: [{ name: "autorelease: pending" }],
  };
  const releases = options.draft
    ? [{ id: 1, tag_name: options.draft, target_commitish: sha, draft: true, prerelease: false }]
    : [{ id: 1, tag_name: "v0.4.1", target_commitish: sha, draft: false, prerelease: false }];
  releases.push({
    id: 2,
    tag_name: "v0.4.0",
    target_commitish: sha,
    draft: false,
    prerelease: false,
  });
  const operations = {
    createReleases: vi.fn<() => Promise<undefined>>(async () => undefined),
    propose: vi.fn<() => Promise<undefined>>(async () => undefined),
  };
  const request: Request = async <T>(path: string): Promise<T> => {
    if (path.startsWith("/pulls?")) return (options.legacy || options.pending ? [pr] : []) as T;
    if (path === "/pulls/94") return pr as T;
    if (path.startsWith("/releases?")) return releases as T;
    if (path.startsWith("/tags?"))
      return [
        { name: "v0.4.1" },
        { name: "v0.4.0" },
        ...(options.orphan ? [{ name: "v0.3.9" }] : []),
      ] as T;
    if (path === "/rules/branches/main")
      return [
        {
          type: "required_status_checks",
          ruleset_id: 1,
          parameters: {
            strict_required_status_checks_policy: options.strict ?? true,
            required_status_checks: [{ context: "code-quality" }],
          },
        },
      ] as T;
    if (path === "/rulesets/1")
      return { enforcement: "active", bypass_actors: options.bypass ? [{}] : [] } as T;
    if (path === "/git/ref/tags/v0.4.1") return { object: { type: "commit", sha } } as T;
    throw new Error(path);
  };
  return { request, operations, releases };
}
describe("release lifecycle guards", () => {
  it.each([false, true])(
    "uses only the policy capability before mutations (pending=%s)",
    async (pending) => {
      const f = fixture({ pending });
      const policyPaths: string[] = [];
      const mutationRequest: Request = async <T>(
        path: string,
        method?: string,
        body?: unknown,
      ): Promise<T> => {
        if (path.startsWith("/rules")) throw new Error("Policy read through mutation credential");
        return f.request<T>(path, method, body);
      };
      const policyRequest = async <T>(path: string): Promise<T> => {
        expect(["/rules/branches/main", "/rulesets/1"]).toContain(path);
        policyPaths.push(path);
        return f.request<T>(path);
      };
      await lifecycle(mutationRequest, policyRequest, ".", repo, sha, f.operations);
      expect(policyPaths).toEqual(["/rules/branches/main", "/rulesets/1"]);
      expect(pending ? f.operations.createReleases : f.operations.propose).toHaveBeenCalledOnce();
    },
  );
  it.each([false, true])(
    "rejects hidden bypass actors before any mutation (pending=%s)",
    async (pending) => {
      const f = fixture({ pending });
      const policyRequest = async <T>(path: string): Promise<T> =>
        path === "/rulesets/1" ? ({ enforcement: "active" } as T) : f.request<T>(path);
      await expect(
        lifecycle(f.request, policyRequest, ".", repo, sha, f.operations),
      ).rejects.toThrow("without bypass actors");
      expect(f.operations.createReleases).not.toHaveBeenCalled();
      expect(f.operations.propose).not.toHaveBeenCalled();
    },
  );
  it("does not propose when strict checks are disabled or bypassable", async () => {
    for (const options of [{ strict: false }, { bypass: true }]) {
      const f = fixture(options);
      await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).rejects.toThrow(
        "strict",
      );
      expect(f.operations.propose).not.toHaveBeenCalled();
      expect(f.operations.createReleases).not.toHaveBeenCalled();
    }
  });
  it("rejects legacy open PRs before mutations", async () => {
    const f = fixture({ legacy: true });
    await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).rejects.toThrow(
      "old-format",
    );
    expect(f.operations.propose).not.toHaveBeenCalled();
  });
  it("resumes an existing draft even without pending labels", async () => {
    const f = fixture({ draft: "v0.4.1", strict: false });
    await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).resolves.toEqual({
      state: "release",
      tag: "v0.4.1",
    });
    expect(f.operations.createReleases).not.toHaveBeenCalled();
  });
  it("rejects a selected release conflicting with the unfinished draft", async () => {
    const f = fixture({ draft: "v0.4.2" });
    await expect(
      lifecycle(f.request, f.request, ".", repo, sha, f.operations, "v0.4.1"),
    ).rejects.toThrow("conflicts");
  });
  it("rejects multiple unfinished releases before mutations", async () => {
    const f = fixture({ draft: "v0.4.1" });
    f.releases.push({
      id: 3,
      tag_name: "v0.4.2",
      target_commitish: sha,
      draft: true,
      prerelease: false,
    });
    await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).rejects.toThrow(
      "Multiple unfinished",
    );
    expect(f.operations.createReleases).not.toHaveBeenCalled();
    expect(f.operations.propose).not.toHaveBeenCalled();
  });
  it("rejects pending PR/draft mismatch", async () => {
    const f = fixture({ pending: true, draft: "v0.4.2" });
    await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).rejects.toThrow(
      /.+/u,
    );
    expect(f.operations.createReleases).not.toHaveBeenCalled();
  });
  it("reobserves duplicate releases and requires the actual tag target", async () => {
    const f = fixture({ pending: true, published: true });
    f.operations.createReleases.mockRejectedValue(
      Object.assign(new Error("duplicate"), { name: "DuplicateReleaseError" }),
    );
    await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).resolves.toEqual({
      state: "release",
      tag: "v0.4.1",
    });
  });
  it("does not admit a duplicate release with a conflicting actual tag", async () => {
    const f = fixture({ pending: true });
    const request: Request = async <T>(path: string): Promise<T> =>
      path.startsWith("/git/ref/")
        ? ({ object: { type: "commit", sha: "b".repeat(40) } } as T)
        : f.request<T>(path);
    f.operations.createReleases.mockRejectedValue(
      Object.assign(new Error("duplicate"), { name: "DuplicateReleaseError" }),
    );
    await expect(lifecycle(request, request, ".", repo, sha, f.operations)).rejects.toThrow(
      "tag does not match",
    );
  });
  it("propagates unrelated creator failures", async () => {
    const f = fixture({ pending: true });
    f.operations.createReleases.mockRejectedValue(new Error("permission denied"));
    await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).rejects.toThrow(
      "permission denied",
    );
  });
  it("rejects orphan tags before proposing", async () => {
    const f = fixture({ orphan: true });
    await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).rejects.toThrow(
      "not published",
    );
    expect(f.operations.propose).not.toHaveBeenCalled();
  });
  it("proposes only after the current release is published", async () => {
    const f = fixture();
    await expect(lifecycle(f.request, f.request, ".", repo, sha, f.operations)).resolves.toEqual({
      state: "proposal",
    });
    expect(f.operations.propose).toHaveBeenCalledOnce();
  });
  it("requires strict checks for an empty initial history", async () => {
    source.bootstrapped = false;
    try {
      const f = fixture();
      const request: Request = async <T>(path: string): Promise<T> =>
        path.startsWith("/tags?") || path.startsWith("/releases?") ? ([] as T) : f.request<T>(path);
      await expect(lifecycle(request, request, ".", repo, sha, f.operations)).resolves.toEqual({
        state: "proposal",
      });
      await expect(requireStrictChecks(fixture({ strict: false }).request)).rejects.toThrow(
        "strict",
      );
    } finally {
      source.bootstrapped = true;
    }
  });
});
