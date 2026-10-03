// Artifact admission remains executable before dependency installation.
import { verifyGeneration as verify } from "adapter-lit-prerenderer/verify";
import { PAGE_ENTRIES } from "../../src/page-entries.ts";
import { readHtmlMathManifest } from "adapter-math-renderer/html-math-manifest";
export const VERIFICATION_CONTRACT = {
  pages: Object.fromEntries(
    Object.entries(PAGE_ENTRIES).map(([view, html]) => [
      view,
      {
        html,
        rootTag: `lens-${view}-view`,
        outlet: `<!-- lens-prerender:${view} -->`,
        nestedRootTags: ["lens-select"],
      },
    ]),
  ),
  resources(directory: string) {
    const math = readHtmlMathManifest(directory);
    return [{ namespace: "assets/html-math/", paths: math.files.map((file) => file.path) }];
  },
};
export function verifyGeneration(directory: string, generation?: string): void {
  verify(directory, VERIFICATION_CONTRACT, generation);
}
