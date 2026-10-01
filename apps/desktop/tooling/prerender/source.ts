import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

export const WORKSPACE_PACKAGE_PATHS = [
  "packages/typescript-config",
  "packages/adapter-mcp-apps-web",
  "packages/adapter-rich-content-web",
] as const;

export const SOURCE_PATHS = [
  ".gitignore",
  "apps/desktop/src",
  "apps/desktop/tooling",
  "apps/desktop/src-tauri/icons",
  "apps/desktop/agent-icons",
  "apps/desktop/package.json",
  "apps/desktop/tsconfig.json",
  "apps/desktop/tsconfig.app.json",
  "apps/desktop/tsconfig.node.json",
  "apps/desktop/tsconfig.test.json",
  "apps/desktop/tsconfig.prerender.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
  "package.json",
  ...WORKSPACE_PACKAGE_PATHS,
];

export function sourceInputs(repository: string): Map<string, Buffer> {
  const stdout = execFileSync(
    "git",
    ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", ...SOURCE_PATHS],
    { cwd: repository, encoding: "utf8" },
  );
  const deleted = new Set(
    execFileSync("git", ["ls-files", "-z", "--deleted", "--", ...SOURCE_PATHS], {
      cwd: repository,
      encoding: "utf8",
    })
      .split("\0")
      .filter(Boolean),
  );
  return new Map(
    [...new Set(stdout.split("\0").filter(Boolean))]
      .filter((file) => !deleted.has(file))
      .sort()
      .map((file) => [file, readFileSync(join(repository, file))]),
  );
}

export function sourceDigest(files: Map<string, Buffer>): string {
  const hash = createHash("sha256");
  for (const [file, bytes] of files) hash.update(file).update("\0").update(bytes).update("\0");
  return hash.digest("hex");
}
