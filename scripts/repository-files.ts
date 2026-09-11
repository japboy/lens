// Repository file enumeration shared by every policy scan. Keeping one implementation is
// what makes the scans agree: the copies they replaced had already drifted, and only the
// workspace-boundary one rejected symbolic links, so the product-identity, language,
// publication and terminology scans read link targets as if they were repository content.

import { execFileSync } from "node:child_process";
import { lstatSync } from "node:fs";
import { resolve } from "node:path";

/// Contents are not decodable text, so a scan reports on the path alone.
export const BINARY_EXTENSIONS = new Set([".icns", ".ico", ".png"]);

/// Fatal decoding keeps a scan from silently reading replacement characters as source.
export const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

/// Tracked and untracked-but-not-ignored paths that exist, deduplicated and sorted.
///
/// Throws when a path is a symbolic link: the link target is outside the enumeration, so
/// scanning it would attribute content to a repository path that does not hold it.
export function repositoryFiles(root: string): string[] {
  const candidates = [
    ...new Set(
      execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "-z"], {
        cwd: root,
        encoding: "utf8",
      })
        .split("\0")
        .filter(Boolean),
    ),
  ].sort();
  // One `lstat` answers both questions this needs — whether the path is still there and
  // whether it is a link — so every scan pays one stat per path rather than two.
  return candidates.filter((path) => {
    let entry;
    try {
      entry = lstatSync(resolve(root, path));
    } catch {
      return false;
    }
    if (entry.isSymbolicLink())
      throw new Error(`Repository paths must not be symbolic links: ${path}`);
    return true;
  });
}
