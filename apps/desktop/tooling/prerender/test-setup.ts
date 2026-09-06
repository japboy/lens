import { BUILD_PATHS } from "../build-paths.ts";
import { resolve } from "node:path";
import { generate } from "./generate.ts";

export default async function setup(): Promise<void> {
  await generate(resolve(BUILD_PATHS.tests));
}
