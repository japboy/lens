import { execFileSync } from "node:child_process";
import { git, headCommit, isCleanCheckout } from "../git.ts";
import { VERSION_FILES, tagVersion, versionState } from "./version.ts";

export { git };
export function commitSha(value: string): string {
  if (!/^[a-f0-9]{40}$/u.test(value)) throw new Error("Explicit full source commit required");
  return value;
}
export function readSource(root: string, sha: string) {
  commitSha(sha);
  const state = versionState(
    Object.fromEntries(VERSION_FILES.map((path) => [path, git(root, "show", `${sha}:${path}`)])),
  );
  return { ...state, sha };
}
export function requireMain(root: string, sha: string): void {
  commitSha(sha);
  execFileSync("git", ["merge-base", "--is-ancestor", sha, "refs/remotes/origin/main"], {
    cwd: root,
    stdio: "pipe",
  });
}
export function cleanSource(root: string, sha: string): void {
  if (headCommit(root) !== commitSha(sha) || !isCleanCheckout(root))
    throw new Error("Expected a clean checkout of the exact release source");
}
export type TagAnnotation = { schema: 1; version: string; commit: string; pullRequest: number };
export function annotation(value: TagAnnotation): string {
  tagVersion(`v${value.version}`);
  commitSha(value.commit);
  if (!Number.isSafeInteger(value.pullRequest) || value.pullRequest < 1 || value.schema !== 1)
    throw new Error("Invalid release annotation");
  return JSON.stringify(value);
}
export function inspectTag(root: string, tag: string): TagAnnotation {
  const version = tagVersion(tag);
  const ref = `refs/tags/${tag}`;
  if (git(root, "cat-file", "-t", ref) !== "tag")
    throw new Error("Lightweight release tags are forbidden");
  const object = git(root, "cat-file", "-p", ref);
  const header = object.split("\n\n")[0]!;
  if (!header.includes("\ntype commit\n") || !header.includes(`\ntag ${tag}\n`))
    throw new Error("Release tag must directly annotate its named commit");
  const payload = object.slice(object.indexOf("\n\n") + 2);
  const data = JSON.parse(payload) as TagAnnotation;
  if (
    annotation(data) !== payload ||
    data.version !== version ||
    data.commit !== git(root, "rev-parse", `${ref}^{commit}`)
  )
    throw new Error("Tag annotation identity mismatch");
  requireMain(root, data.commit);
  const source = readSource(root, data.commit);
  if (!source.bootstrapped || source.version !== version)
    throw new Error("Tag/source version mismatch");
  return data;
}
export function changelogSection(source: string, version: string): string {
  tagVersion(`v${version}`);
  const headings = [...source.matchAll(/^## (?:\[)?(\d+\.\d+\.\d+)(?:\])?(?:[^\n]*)$/gmu)];
  const matches = headings.filter((heading) => heading[1] === version);
  if (matches.length !== 1 || matches[0] !== headings[0])
    throw new Error("Expected exactly one newest changelog section for the release version");
  const section = source.slice(matches[0]!.index, headings[1]?.index ?? source.length).trim();
  if (!section.split("\n").slice(1).join("\n").trim())
    throw new Error("Release changelog is empty");
  return section;
}
export function assertPrTitle(title: string): void {
  if (
    !/^(?:feat|fix|perf|revert|docs|chore|ci|test|refactor|build|style)(?:\([a-zA-Z0-9._/-]+\))?!?: [^\r\n]+$/u.test(
      title,
    )
  )
    throw new Error("PR title must be a Conventional Commit squash message");
}
