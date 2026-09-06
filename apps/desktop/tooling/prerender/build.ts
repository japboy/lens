import { resolve } from "node:path";
import { generate } from "./generate.ts";

const development = process.argv.includes("--development");
const output = process.argv
  .find((argument) => argument.startsWith("--output="))
  ?.slice("--output=".length);
const generation = await generate(resolve(output ?? "dist"), development);
console.log(`Generated all WebViews: ${generation}`);
