import { resolve } from "node:path";
import { startDevelopmentServer } from "adapter-lit-prerenderer";
import { workspacePackagePaths } from "./source.ts";
import { APPLICATION_ROOT, REPOSITORY_ROOT } from "./contract.ts";
import { BUILD_PATHS } from "../build-paths.ts";
import { PAGE_ENTRIES } from "../../src/page-entries.ts";
import {
  readHtmlMathManifest,
  htmlMathResponseHeaders,
} from "adapter-math-renderer/html-math-manifest";
const server = await startDevelopmentServer({
  outputRoot: resolve(APPLICATION_ROOT, BUILD_PATHS.development),
  pages: Object.values(PAGE_ENTRIES),
  port: 1420,
  host: "127.0.0.1",
  buildCommand: {
    command: process.execPath,
    arguments: [resolve(APPLICATION_ROOT, "tooling/prerender/build.ts"), "--development"],
    cwd: APPLICATION_ROOT,
  },
  watchRoots: [
    {
      directory: APPLICATION_ROOT,
      accepts: (path) =>
        /^(src\/|tooling\/|agent-icons\/|src-tauri\/icons\/|package\.json$|tsconfig[^/]*\.json$)/.test(
          path,
        ),
    },
    {
      directory: REPOSITORY_ROOT,
      accepts: (path) =>
        [
          ".gitignore",
          "pnpm-lock.yaml",
          "pnpm-workspace.yaml",
          "package.json",
          "scripts/workspace-policy.ts",
        ].includes(path),
    },
    ...workspacePackagePaths(REPOSITORY_ROOT).map((directory) => ({
      directory: resolve(REPOSITORY_ROOT, directory),
      accepts: (path: string) =>
        !path.split("/").some((part) => part === "node_modules" || part === ".build"),
    })),
  ],
  readResources: readHtmlMathManifest,
  resourceHeaders: htmlMathResponseHeaders,
});
for (const signal of ["SIGINT", "SIGTERM"] as const)
  process.on(signal, () => {
    void server.close();
  });
