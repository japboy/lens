#!/usr/bin/env node
//MISE description = "Run bounded on-device native probes"
//MISE dir = "{{config_root}}"

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

interface ProbeResult {
  status: "passed" | "failed" | "permission_required";
}

interface Probe {
  sources: readonly string[];
  binary: string;
  prefix: string;
  frameworks: readonly string[];
  definitions?: readonly string[];
}

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const temporaryRoot = mkdtempSync(join(tmpdir(), "lens-live-sync-native-"));
const nativeRoot = join(repositoryRoot, "packages", "adapter-platform-macos", "native");
const compileTimeoutMilliseconds = 60_000;
const probeTimeoutMilliseconds = 15_000;
const probes: readonly Probe[] = [
  {
    sources: ["LensAXObserverProbe.m"],
    binary: "lens-ax-observer-probe",
    prefix: "LENS_AX_OBSERVER_PROBE=",
    frameworks: ["AppKit", "ApplicationServices"],
  },
  {
    sources: ["LensWindowCaptureProbe.m"],
    binary: "lens-retained-window-capture-probe",
    prefix: "LENS_SCWINDOW_PROBE=",
    frameworks: ["AppKit", "CoreGraphics", "ScreenCaptureKit"],
  },
  {
    sources: ["LensNativeIntegrationProbe.m", "LensNative.m"],
    binary: "lens-native-integration-probe",
    prefix: "LENS_NATIVE_INTEGRATION_PROBE=",
    frameworks: ["AppKit", "ApplicationServices", "CoreGraphics", "ScreenCaptureKit"],
    definitions: ["LENS_NATIVE_TESTING=1"],
  },
];

function runProbe(probe: Probe): number {
  const probeBinary = join(temporaryRoot, probe.binary);
  const frameworkArguments = probe.frameworks.flatMap((framework) => ["-framework", framework]);
  const compile = spawnSync(
    "xcrun",
    [
      "clang",
      "-fobjc-arc",
      "-fblocks",
      "-Wall",
      "-Wextra",
      "-Werror",
      "-mmacosx-version-min=15.2",
      "-I",
      nativeRoot,
      ...(probe.definitions ?? []).map((definition) => `-D${definition}`),
      ...frameworkArguments,
      ...probe.sources.map((source) => join(nativeRoot, source)),
      "-o",
      probeBinary,
    ],
    {
      cwd: repositoryRoot,
      stdio: "inherit",
      timeout: compileTimeoutMilliseconds,
      killSignal: "SIGKILL",
    },
  );
  if (compile.error) {
    throw compile.error;
  }
  if (compile.status !== 0) {
    return compile.status ?? 1;
  }

  const execution = spawnSync(probeBinary, [], {
    cwd: repositoryRoot,
    encoding: "utf8",
    timeout: probeTimeoutMilliseconds,
    killSignal: "SIGKILL",
  });
  if (execution.error) {
    const errorCode = (execution.error as NodeJS.ErrnoException).code;
    console.error(
      errorCode === "ETIMEDOUT"
        ? `${probe.sources.join(" + ")} did not terminate within the finite runner boundary.`
        : `${probe.sources.join(" + ")} execution failed${errorCode === undefined ? "" : ` (${errorCode})`}.`,
    );
    console.error(execution.error.message);
    return 1;
  }
  process.stdout.write(execution.stdout);
  process.stderr.write(execution.stderr);

  const resultLine = execution.stdout.split(/\r?\n/u).find((line) => line.startsWith(probe.prefix));
  if (resultLine === undefined) {
    console.error(`${probe.sources.join(" + ")} returned no structured result.`);
    return 1;
  }
  const result = JSON.parse(resultLine.slice(probe.prefix.length)) as ProbeResult;
  if (result.status !== "passed") {
    return execution.status !== null && execution.status !== 0 ? execution.status : 1;
  }
  if (execution.status !== 0) {
    return execution.status ?? 1;
  }
  return 0;
}

function run(): number {
  if (process.platform !== "darwin") {
    console.error("Live-sync native validation requires macOS.");
    return 1;
  }
  for (const probe of probes) {
    const status = runProbe(probe);
    if (status !== 0) {
      return status;
    }
  }
  return 0;
}

try {
  process.exitCode = run();
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}
