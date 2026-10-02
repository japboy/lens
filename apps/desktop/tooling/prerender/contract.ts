import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { linkGenerationDependencies, type PrerenderContract } from "adapter-lit-prerenderer";
import { workspaceDirectories } from "adapter-lit-prerenderer/source-snapshot";
import { BUILD_PATHS, assertGenerationOutput } from "../build-paths.ts";
import { sourceInputs, sourceContract } from "./source.ts";
import { VERIFICATION_CONTRACT, verifyGeneration } from "./verify.ts";
import { htmlMathAssetsPlugin } from "../html-math-assets.ts";
export const APPLICATION_ROOT = fileURLToPath(new URL("../../", import.meta.url));
export const REPOSITORY_ROOT = resolve(APPLICATION_ROOT, "../..");
export const PRERENDER_CONTRACT: PrerenderContract = {
  applicationPath: "apps/desktop",
  clientPath: "src",
  stagingRoot: resolve(APPLICATION_ROOT, BUILD_PATHS.staging),
  renderEntry: "tooling/prerender/render-entry.ts",
  rendererBundleFilename: "render-entry.js",
  pages: VERIFICATION_CONTRACT.pages,
  sourceInputs: () => sourceInputs(REPOSITORY_ROOT),
  linkDependencies: (staging) =>
    linkGenerationDependencies(
      REPOSITORY_ROOT,
      staging,
      workspaceDirectories(sourceContract(REPOSITORY_ROOT)),
    ),
  plugins: async (app) => [await htmlMathAssetsPlugin(app)],
  verify: verifyGeneration,
  assertOutput: (output) => assertGenerationOutput(APPLICATION_ROOT, output),
};
