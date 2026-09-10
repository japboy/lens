import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { refreshCargoLock } from "./cargo-lock.ts";
import { PENDING, RELEASE_BRANCH } from "./control.ts";
import { pages } from "./github.ts";
import type { Request } from "./github.ts";
import { cleanSource, commitSha, git, requireMain } from "./source.ts";
import { compareVersions, manifestVersionState, readVersion, VERSION_FILES } from "./version.ts";

type ReleaseProposal = {
  number: number;
  state: string;
  merged: boolean;
  labels: { name: string }[];
  head: { ref: string; sha: string; repo: { full_name: string } | null };
  base: { ref: string; repo: { full_name: string } };
};

function requireProposal(pr: ReleaseProposal, number: number, repository: string): string {
  if (
    pr.number !== number ||
    pr.state !== "open" ||
    pr.merged ||
    pr.base.ref !== "main" ||
    pr.base.repo.full_name !== repository ||
    pr.head.repo?.full_name !== repository ||
    pr.head.ref !== RELEASE_BRANCH ||
    !pr.labels.some((label) => label.name === PENDING)
  )
    throw new Error("Expected the open repository-owned pending release PR");
  return commitSha(pr.head.sha);
}

export async function refreshReleaseLock(
  request: Request,
  root: string,
  sourceSha: string,
  repository: string,
  refresh: (baselineRoot: string, candidateRoot: string) => string = refreshCargoLock,
): Promise<"absent" | "unchanged" | "updated"> {
  cleanSource(root, sourceSha);
  requireMain(root, sourceSha);
  const proposals = await pages<{ number: number }>(
    request,
    `/pulls?state=open&base=main&head=${encodeURIComponent(`${repository.split("/")[0]}:${RELEASE_BRANCH}`)}`,
  );
  if (!proposals.length) return "absent";
  if (proposals.length !== 1) throw new Error("Expected at most one open release PR");
  const number = proposals[0]!.number;
  if (!Number.isSafeInteger(number) || number < 1) throw new Error("Invalid release PR number");
  const head = requireProposal(await request(`/pulls/${number}`), number, repository);
  git(root, "fetch", "--quiet", "--no-tags", "origin", `refs/pull/${number}/head`);
  if (git(root, "rev-parse", "FETCH_HEAD") !== head)
    throw new Error("Release PR changed while fetching its head");
  git(root, "merge-base", "--is-ancestor", sourceSha, head);

  // Only version data may differ from the trusted event source. Never run candidate code.
  const allowed = new Set<string>([...VERSION_FILES, "CHANGELOG.md"]);
  const changes = git(root, "diff", "--raw", "--no-renames", sourceSha, head);
  for (const change of changes ? changes.split("\n") : []) {
    const [metadata, path] = change.split("\t");
    const fields = metadata!.split(" ");
    const [oldMode, newMode, , , status] = fields;
    if (
      !path ||
      !allowed.has(path) ||
      newMode !== "100644" ||
      !(
        (oldMode === ":100644" && status === "M") ||
        (oldMode === ":000000" && status === "A" && path === "CHANGELOG.md")
      )
    )
      throw new Error("Release PR may change only existing version files and its changelog");
  }

  const temporary = mkdtempSync(join(tmpdir(), "lens-refresh-release-lock-"));
  const candidate = join(temporary, "candidate");
  let attached = false;
  try {
    execFileSync(
      "git",
      ["-c", "core.hooksPath=/dev/null", "worktree", "add", "--detach", candidate, head],
      {
        cwd: root,
        stdio: "pipe",
      },
    );
    attached = true;
    const baseline = readVersion(root);
    const proposed = manifestVersionState(
      Object.fromEntries(
        VERSION_FILES.map((path) => [path, readFileSync(join(candidate, path), "utf8")]),
      ),
    );
    if (
      !proposed.bootstrapped ||
      !(
        compareVersions(proposed.version, baseline.version) > 0 ||
        (!baseline.bootstrapped && baseline.version === "0.1.0" && proposed.version === "0.1.0")
      )
    )
      throw new Error("Release proposal must advance the product version or bootstrap 0.1.0");
    const original = readFileSync(join(candidate, "Cargo.lock"), "utf8");
    const lock = refresh(root, candidate);
    writeFileSync(join(candidate, "Cargo.lock"), lock);
    readVersion(candidate);
    const status = git(candidate, "status", "--porcelain", "--untracked-files=all");
    if (status !== "" && status !== " M Cargo.lock")
      throw new Error("Cargo refresh may modify only Cargo.lock");
    if (lock === original) return "unchanged";

    const blob = await request<{ sha: string }>("/git/blobs", "POST", {
      content: lock,
      encoding: "utf-8",
    });
    const tree = await request<{ sha: string }>("/git/trees", "POST", {
      base_tree: git(root, "rev-parse", `${head}^{tree}`),
      tree: [{ path: "Cargo.lock", mode: "100644", type: "blob", sha: commitSha(blob.sha) }],
    });
    const commit = await request<{ sha: string }>("/git/commits", "POST", {
      message: "chore: refresh workspace release lockfile",
      tree: commitSha(tree.sha),
      parents: [head],
    });
    if (requireProposal(await request(`/pulls/${number}`), number, repository) !== head)
      throw new Error("Release PR changed while refreshing its lockfile");
    // A concurrent advance after the recheck cannot fast-forward to this sibling commit.
    await request(`/git/refs/heads/${RELEASE_BRANCH}`, "PATCH", {
      sha: commitSha(commit.sha),
      force: false,
    });
    return "updated";
  } finally {
    try {
      if (attached) git(root, "worktree", "remove", "--force", candidate);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  }
}
