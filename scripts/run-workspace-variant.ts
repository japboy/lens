import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { inspectFeatureGraphs } from "./workspace-feature-graphs.ts";
import { BUILD_VARIANTS, variantArguments } from "./workspace-policy.ts";

type Report = ReturnType<typeof inspectFeatureGraphs>;
type Admission = {
  version: number;
  rustcRelease: string;
  hosts: Record<string, Record<string, { graphDigest: string; arguments: string[] }>>;
};
const ROOT = fileURLToPath(new URL("..", import.meta.url));

export function validateVariantAdmission(report: Report, admission: Admission): void {
  if (admission.version !== 1 || !/^\d+\.\d+\.\d+$/u.test(admission.rustcRelease))
    throw new Error("Unsupported variant admission policy");
  const release = /^release: (.+)$/mu.exec(report.rustc)?.[1];
  if (release !== admission.rustcRelease) throw new Error("Unreviewed compiler release");
  const host = admission.hosts[report.host];
  if (!host) throw new Error("Unreviewed compiler host");
  if (
    Object.keys(host).length !== BUILD_VARIANTS.length ||
    !BUILD_VARIANTS.every((variant) => host[variant.id])
  )
    throw new Error("Incomplete host variant admission");
  if (
    !report.variants.length ||
    new Set(report.variants.map((variant) => variant.variant)).size !== report.variants.length
  )
    throw new Error("Empty or duplicate variant analysis");
  for (const observed of report.variants) {
    const variant = BUILD_VARIANTS.find((entry) => entry.id === observed.variant);
    const expected = host[observed.variant];
    if (
      !variant ||
      !expected ||
      observed.profile !== variant.profile ||
      JSON.stringify(expected.arguments) !== JSON.stringify(variantArguments(variant))
    )
      throw new Error("Unreviewed package/target/features/profile invocation");
    if (!/^[a-f0-9]{64}$/u.test(expected.graphDigest) || observed.digest !== expected.graphDigest)
      throw new Error(`Unreviewed dependency/feature graph: ${report.host}/${observed.variant}`);
  }
}

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
  const admission: Admission = JSON.parse(
    readFileSync(resolve(root, "scripts/workspace-variants.json"), "utf8"),
  );
  validateVariantAdmission(report, admission);
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
