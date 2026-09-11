// Git queries shared by the release pipeline, CI change classification and the frontend
// artifact manifest. All three need to know that they are reading an exact, unmodified
// checkout, and each had grown its own spelling of that question.

import { execFileSync } from "node:child_process";

/// Runs git in `root` and returns its trimmed stdout as text.
export function git(root: string, ...args: string[]): string {
  return execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  }).trimEnd();
}

/// The commit `HEAD` currently resolves to.
export function headCommit(root: string): string {
  return git(root, "rev-parse", "HEAD");
}

/// Whether the working tree matches `HEAD`, counting untracked files that are not ignored.
///
/// A build or a classification that reads a modified tree describes something that was
/// never committed, so every caller checks this before recording what it saw.
export function isCleanCheckout(root: string): boolean {
  return !git(root, "status", "--porcelain", "--untracked-files=all");
}
