import { resolve } from "node:path";
import { startDevelopmentServer } from "adapter-lit-prerenderer";
import { sourceWatchRoots } from "./source.ts";
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
  watchRoots: sourceWatchRoots(REPOSITORY_ROOT),
  readResources: readHtmlMathManifest,
  resourceHeaders: htmlMathResponseHeaders,
});
for (const signal of ["SIGINT", "SIGTERM"] as const)
  process.on(signal, () => {
    void server.close();
  });
