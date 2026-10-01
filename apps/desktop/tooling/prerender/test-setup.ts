import { fileURLToPath } from "node:url";
import { resolve } from "node:path";
import { BUILD_PATHS } from "../build-paths.ts";
import { generate } from "./generate.ts";

const desktop = fileURLToPath(new URL("../../", import.meta.url));
export default async function setup(): Promise<void> {
  await generate(resolve(desktop, BUILD_PATHS.tests));
}
