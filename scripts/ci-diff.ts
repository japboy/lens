import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseChangedPaths, planChanges } from "./ci-plan.ts";
import type { Change } from "./ci-plan.ts";

const decoder = new TextDecoder("utf-8", { fatal: true });
export function planGitChanges(root: string, base: string, head: string) {
  if (!/^[a-f0-9]{40}$/u.test(base) || !/^[a-f0-9]{40}$/u.test(head))
    throw new Error("Exact base and tested commit SHAs are required");
  const git = (args: string[]) =>
    execFileSync("git", ["--literal-pathspecs", ...args], {
      cwd: root,
      maxBuffer: 32 * 1024 * 1024,
    });
  for (const sha of [base, head]) {
    if (decoder.decode(git(["rev-parse", "--verify", `${sha}^{commit}`])).trim() !== sha)
      throw new Error("Unexpected resolved commit");
  }
  if (decoder.decode(git(["rev-parse", "HEAD"])).trim() !== head)
    throw new Error("Classification must use the actual tested checkout");
  if (git(["status", "--porcelain", "-z", "--untracked-files=all"]).length)
    throw new Error("Classification cannot use a modified source checkout");
  git(["merge-base", "--is-ancestor", base, head]);
  const paths = parseChangedPaths(
    decoder.decode(git(["diff", "--no-renames", "--name-only", "-z", base, head, "--"])),
  );
  const readAt = (sha: string, path: string): string | null => {
    const entry = decoder.decode(git(["ls-tree", "-z", sha, "--", path]));
    if (!entry) return null;
    const match = /^(100644|100755) blob ([a-f0-9]{40})\t([^\0]+)\0$/u.exec(entry);
    if (!match || match[3] !== path) throw new Error(`Unreviewed changed object: ${path}`);
    // Only shared Rust needs textual body comparison; binary frontend resources must
    // not be decoded as source. Mode/object ownership is still checked for every path.
    return /^packages\/(?:domain|usecase)\/src\/.+\.rs$/u.test(path)
      ? decoder.decode(git(["cat-file", "blob", match[2]!]))
      : "";
  };
  const changes: Change[] = paths.map((path) => ({
    path,
    before: readAt(base, path),
    after: readAt(head, path),
  }));
  return { base, head, plan: planChanges(changes), paths };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length !== 2 || !process.env.GITHUB_OUTPUT)
    throw new Error("Explicit CI environment is required");
  const result = planGitChanges(
    fileURLToPath(new URL("..", import.meta.url)),
    process.env.BASE_SHA ?? "",
    process.env.HEAD_SHA ?? "",
  );
  appendFileSync(process.env.GITHUB_OUTPUT, `plan=${result.plan}\n`);
  process.stdout.write(`${JSON.stringify(result)}\n`);
}
