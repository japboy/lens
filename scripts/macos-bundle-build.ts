import { BUILD_PATHS } from "../apps/desktop/tooling/build-paths.ts";
import { sourceDigest, sourceInputs } from "../apps/desktop/tooling/prerender/source.ts";
import { verifyGeneration } from "../apps/desktop/tooling/prerender/verify.ts";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { bundleContract } from "./release/bundle.ts";
import { runVariant } from "./run-workspace-variant.ts";

// Cache seeding and packaging must compile precisely the same admitted variant.
export function buildMacosBundle(root: string) {
  if (process.platform !== "darwin" || process.arch !== "arm64")
    throw new Error("Packaged compilation requires the admitted Apple-silicon host");
  const contract = bundleContract(root);
  const previous = process.env.MACOSX_DEPLOYMENT_TARGET;
  if (previous !== undefined && previous !== contract.minimum)
    throw new Error("The macOS deployment environment differs from the bundle contract");
  verifyGeneration(
    join(root, "apps/desktop", BUILD_PATHS.webview),
    sourceDigest(sourceInputs(root)),
  );
  try {
    process.env.MACOSX_DEPLOYMENT_TARGET = contract.minimum;
    runVariant("macos-bundle-build", root);
  } finally {
    if (previous === undefined) delete process.env.MACOSX_DEPLOYMENT_TARGET;
    else process.env.MACOSX_DEPLOYMENT_TARGET = previous;
  }
  return contract;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length !== 2) throw new Error("The packaged macOS build accepts no arguments");
  buildMacosBundle(fileURLToPath(new URL("..", import.meta.url)));
}
