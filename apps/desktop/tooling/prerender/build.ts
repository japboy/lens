import { BUILD_PATHS } from "../build-paths.ts";
import { resolve } from "node:path";
import { generate } from "./generate.ts";

const development = process.argv.includes("--development");
const output = process.argv
  .find((argument) => argument.startsWith("--output="))
  ?.slice("--output=".length);
const generation = await generate(resolve(output ?? BUILD_PATHS.webview), development);
console.log(`Generated all WebViews: ${generation}`);
