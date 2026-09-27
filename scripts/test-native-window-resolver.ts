import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const nativeRoot = join(repositoryRoot, "packages/adapter-platform-macos/native");
const temporaryRoot = mkdtempSync(join(tmpdir(), "lens-window-resolver-tests-"));
const binary = join(temporaryRoot, "window-resolver-tests");

function run(command: string, args: readonly string[], timeout: number): number {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    stdio: "inherit",
    timeout,
    killSignal: "SIGKILL",
  });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

try {
  const compiled = run(
    "xcrun",
    [
      "clang",
      "-fobjc-arc",
      "-fblocks",
      "-Wall",
      "-Wextra",
      "-Werror",
      "-mmacosx-version-min=15.2",
      ...["AppKit", "ApplicationServices", "CoreGraphics", "ScreenCaptureKit"].flatMap(
        (framework) => ["-framework", framework],
      ),
      join(nativeRoot, "LensWindowResolverTests.m"),
      "-o",
      binary,
    ],
    60_000,
  );
  process.exitCode = compiled === 0 ? run(binary, [], 15_000) : compiled;
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}
