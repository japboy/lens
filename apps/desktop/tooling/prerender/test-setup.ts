import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { BUILD_PATHS } from "../build-paths.ts";
import { generateGeneration } from "adapter-lit-prerenderer";
import { PRERENDER_CONTRACT } from "./contract.ts";

const desktop = fileURLToPath(new URL("../../", import.meta.url));
export default async function setup(): Promise<void> {
  await generateGeneration(PRERENDER_CONTRACT, resolve(desktop, BUILD_PATHS.tests));
}
