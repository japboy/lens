import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

export const SOURCE_PATHS = [
  "apps/desktop/src",
  "apps/desktop/tooling",
  "apps/desktop/src-tauri/icons",
  "apps/desktop/package.json",
  "apps/desktop/tsconfig.json",
  "apps/desktop/tsconfig.app.json",
  "apps/desktop/tsconfig.node.json",
  "apps/desktop/tsconfig.test.json",
  "apps/desktop/tsconfig.prerender.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
  "package.json",
  "packages/typescript-config",
];

export function sourceInputs(repository: string): Map<string, Buffer> {
  const stdout = execFileSync(
    "git",
    ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", ...SOURCE_PATHS],
    { cwd: repository, encoding: "utf8" },
  );
  return new Map(
    [...new Set(stdout.split("\0").filter(Boolean))]
      .sort()
      .map((file) => [file, readFileSync(join(repository, file))]),
  );
}

export function sourceDigest(files: Map<string, Buffer>): string {
  const hash = createHash("sha256");
  for (const [file, bytes] of files) hash.update(file).update("\0").update(bytes).update("\0");
  return hash.digest("hex");
}
