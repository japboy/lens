import type { Plugin } from "vite";
import { createHtmlMathAssets, inlineHtmlMathCss } from "adapter-math-renderer/node";
import { HTML_MATH_MANIFEST } from "adapter-math-renderer/html-math-manifest";

/** Desktop owns virtual-module naming and Vite publication, not resource generation. */
export async function htmlMathAssetsPlugin(buildRoot: string): Promise<Plugin> {
  const { manifest, sources } = await createHtmlMathAssets(buildRoot);
  const inlineCss = inlineHtmlMathCss(manifest, sources);
  const name = "virtual:lens-html-math-assets";
  let building = false;
  return {
    name: "lens-html-math-assets",
    configResolved(config) {
      building = config.command === "build";
    },
    resolveId(id) {
      if (id === name) return `\0${name}`;
    },
    load(id) {
      if (id === `\0${name}`)
        return `export const htmlMathAssets = ${JSON.stringify(manifest)}; export const htmlMathInlineCss = ${JSON.stringify(inlineCss)};`;
    },
    buildStart() {
      if (!building) return;
      for (const [fileName, source] of sources) this.emitFile({ type: "asset", fileName, source });
      this.emitFile({
        type: "asset",
        fileName: HTML_MATH_MANIFEST,
        source: JSON.stringify(manifest) + "\n",
      });
    },
  };
}
