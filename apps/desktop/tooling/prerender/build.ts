import { BUILD_PATHS } from "../build-paths.ts";
import { resolve } from "node:path";
import { generateGeneration } from "adapter-lit-prerenderer";
import { APPLICATION_ROOT, PRERENDER_CONTRACT } from "./contract.ts";

const development = process.argv.includes("--development");
const output = process.argv
  .find((argument) => argument.startsWith("--output="))
  ?.slice("--output=".length);
const generation = await generateGeneration(
  PRERENDER_CONTRACT,
  output ? resolve(output) : resolve(APPLICATION_ROOT, BUILD_PATHS.webview),
  development,
);
console.log(`Generated all WebViews: ${generation}`);
