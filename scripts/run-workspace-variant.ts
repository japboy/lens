import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { inspectFeatureGraphs } from "../mise-tasks/inspect/features.ts";
import { BUILD_VARIANTS, variantArguments } from "./workspace-policy.ts";

const ROOT = fileURLToPath(new URL("..", import.meta.url));

export function selectVariant(id: string) {
  const variant = BUILD_VARIANTS.find((entry) => entry.id === id);
  if (!variant) throw new Error(`Unknown workspace variant: ${id}`);
  return variant;
}

export function assertBuildEnvironment(environment: NodeJS.ProcessEnv): void {
  for (const [name, value] of Object.entries(environment)) {
    if (
      value &&
      /^(?:RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|CARGO_BUILD_RUSTFLAGS|CARGO_TARGET_.+_RUSTFLAGS)$/u.test(
        name,
      )
    )
      throw new Error(`Unreviewed compiler override: ${name}`);
  }
}

export function runVariant(id: string, root = ROOT): void {
  const variant = selectVariant(id);
  assertBuildEnvironment(process.env);
  const report = inspectFeatureGraphs(root, [variant]);
  // Only the portable Apple libraries can be cross-checked without native SDKs.
  if (report.host !== variant.target && variant.id !== "apple-portable-check")
    throw new Error("Native/common-shell variants require their declared host");
  const args = variantArguments(variant);
  process.stdout.write(
    `${JSON.stringify({ variant: id, host: report.host, arguments: args, graphDigest: report.variants[0]!.digest })}\n`,
  );
  execFileSync("cargo", args, { cwd: root, stdio: "inherit" });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length !== 3) throw new Error("Exactly one declared variant ID is required");
  runVariant(process.argv[2]!);
}
