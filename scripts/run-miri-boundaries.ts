import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  MIRI_EXECUTION,
  MIRI_GROUPS,
  RUST_SAFETY_BOUNDARIES,
  NATIVE_ONLY_TESTS,
} from "./rust-safety-boundaries.ts";

export function selectedMiriTests(): string[] {
  const names = Object.values(MIRI_GROUPS).flatMap((group) => [...group.tests]);
  if (names.length === 0 || new Set(names).size !== names.length)
    throw new Error("Miri selection must declare a nonempty unique production test inventory");
  for (const [id, boundary] of Object.entries(RUST_SAFETY_BOUNDARIES)) {
    if (!boundary.conditions.length || !boundary.normalTests.length || !boundary.nativeGap.trim())
      throw new Error(`Incomplete safety conditions or ordinary/native evidence: ${id}`);
    for (const group of boundary.miriGroups)
      if (!MIRI_GROUPS[group].tests.length) throw new Error(`Empty Miri group: ${id}`);
  }
  for (const nativeTest of NATIVE_ONLY_TESTS) {
    if (!nativeTest.command || !nativeTest.reason.trim() || !nativeTest.boundaries.length)
      throw new Error(
        "Native-only boundary tests require explicit command and unsupported-effect reason",
      );
  }
  return names.sort();
}

export function validateMiriSelection(output: string, expected: readonly string[]): void {
  const selected = [...output.matchAll(/^([^\r\n]+): test$/gmu)].map((match) => match[1]!).sort();
  if (!expected.length || JSON.stringify(selected) !== JSON.stringify([...expected].sort()))
    throw new Error(
      `Miri test inventory changed; expected ${expected.length}, selected ${selected.length}: ${selected.join(", ")}`,
    );
}

export function validateMiriResult(output: string, expectedCount: number): void {
  const summary = /test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;/u.exec(output);
  if (!summary || Number(summary[1]) !== expectedCount || summary[2] !== "0" || summary[3] !== "0")
    throw new Error("Miri must execute every declared test successfully without ignored tests");
}

export function runMiriBoundaries(): void {
  const expected = selectedMiriTests();
  const groups = Object.values(MIRI_GROUPS);
  const packages = [...new Set(groups.map((group) => group.package))];
  if (packages.length !== 1) throw new Error("Review Miri batching before adding another package");
  const filters = groups.map((group) => group.filter);
  const common = [
    `+${MIRI_EXECUTION.toolchain}`,
    "miri",
    "test",
    "--locked",
    "-p",
    packages[0]!,
    "--lib",
    "--",
  ];
  const env = {
    ...process.env,
    MIRIFLAGS: `-Zmiri-seed=${MIRI_EXECUTION.seed}`,
    CARGO_TARGET_DIR:
      process.env.CARGO_TARGET_DIR ??
      resolve(tmpdir(), `lens-miri-${MIRI_EXECUTION.toolchain}-target`),
  };
  // Explicit target selects interpreted platform and never changes rustup defaults.
  common.splice(common.indexOf("--lib"), 0, "--target", MIRI_EXECUTION.target);
  const invoke = (args: string[]) => {
    const started = performance.now();
    const result = spawnSync("cargo", args, {
      cwd: fileURLToPath(new URL("../", import.meta.url)),
      env,
      encoding: "utf8",
      timeout: MIRI_EXECUTION.timeoutMs,
      maxBuffer: 16 * 1024 * 1024,
    });
    process.stdout.write(result.stdout ?? "");
    process.stderr.write(result.stderr ?? "");
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`Miri command failed (${result.status})`);
    return { output: result.stdout, wallSeconds: (performance.now() - started) / 1000 };
  };
  const discovery = invoke([...common, "--list", ...filters]);
  validateMiriSelection(discovery.output, expected);
  const execution = invoke([...common, "--test-threads=1", ...filters]);
  validateMiriResult(execution.output, expected.length);
  process.stdout.write(
    JSON.stringify({
      toolchain: MIRI_EXECUTION.toolchain,
      target: MIRI_EXECUTION.target,
      seed: MIRI_EXECUTION.seed,
      tests: expected.length,
      discoveryWallSeconds: discovery.wallSeconds,
      executionWallSeconds: execution.wallSeconds,
    }) + "\n",
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1]))
  runMiriBoundaries();
