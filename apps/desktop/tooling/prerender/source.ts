import {
  sourceInputs as captureSource,
  sourceDigest,
  sourcePaths,
  type SourceSnapshotContract,
} from "adapter-lit-prerenderer/source-snapshot";
import { resolve } from "node:path";
import { MEMBERS } from "../../../../scripts/workspace-policy.ts";

const APPLICATION_SOURCE_PATHS = [
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
  "scripts/workspace-policy.ts",
  "scripts/no-install-workspace.ts",
];

export function sourceContract(repository: string): SourceSnapshotContract {
  return {
    repository,
    applicationPath: "apps/desktop",
    members: MEMBERS.filter((member) => member.ecosystem === "pnpm"),
    inputs: APPLICATION_SOURCE_PATHS,
  };
}
export function sourceInputs(repository: string): Map<string, Buffer> {
  return captureSource(sourceContract(repository));
}
export { sourceDigest };
/** The same admitted path scopes drive source identity and development rebuilds. */
export function sourceWatchRoots(repository: string) {
  const paths = sourcePaths(sourceContract(repository));
  return [
    {
      directory: resolve(repository),
      accepts: (path: string) =>
        !path.split("/").some((part) => part === "node_modules" || part === ".build") &&
        paths.some((scope) => path === scope || path.startsWith(`${scope}/`)),
    },
  ];
}
