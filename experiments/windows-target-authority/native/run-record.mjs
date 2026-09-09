import { createHash } from "node:crypto";
import { open } from "node:fs/promises";
import { release } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { superviseAcquisition } from "./supervisor.mjs";

const nativeDirectory = dirname(fileURLToPath(import.meta.url));
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

// Bound every read before allocation, including an extra byte to detect growth.
async function boundedRead(path, cap) {
  const file = await open(path, "r");
  try {
    const stat = await file.stat();
    if (!stat.isFile() || stat.size > cap) throw new Error("Provenance input exceeds bounds");
    const bytes = Buffer.alloc(stat.size + 1);
    let length = 0;
    while (length < bytes.length) {
      const read = await file.read(bytes, length, bytes.length - length, null);
      if (!read.bytesRead) break;
      length += read.bytesRead;
    }
    if (length !== stat.size) throw new Error("Provenance input changed while reading");
    return bytes.subarray(0, length);
  } finally {
    await file.close();
  }
}

// Validate the provenance fields consumed by acquisition, not the complete smoke
// executable metadata schema. Unconsumed build fields remain reported metadata.
export function validateBuildRecord(record) {
  if (
    !record ||
    record.version !== 3 ||
    record.scope !== "sdk-smoke-fixture-and-acquisition-probe-build-only" ||
    typeof record.source_sha !== "string" ||
    !/^[0-9a-f]{40}$/.test(record.source_sha) ||
    typeof record.worktree_dirty !== "boolean" ||
    record.fixture_executed !== false ||
    record.probe_executed !== false ||
    record.executable?.architecture !== "x64" ||
    record.executable?.native_acquisition_executed !== false ||
    record.executable?.product_admission_granted !== false
  )
    throw new Error("Invalid build provenance");
  for (const key of [
    "fixture_source_sha256",
    "fixture_binary_sha256",
    "probe_source_sha256",
    "probe_binary_sha256",
    "script_sha256",
    "compiler_sha256",
  ]) {
    if (typeof record[key] !== "string" || !/^[0-9a-f]{64}$/.test(record[key]))
      throw new Error("Invalid build digest");
  }
  for (const key of ["sdk_version", "toolset_version", "os_version", "os_build"]) {
    if (typeof record[key] !== "string" || record[key].length < 1 || record[key].length > 256)
      throw new Error("Invalid build environment");
  }
  return record;
}

/** Platform-neutral file verification; never starts a process or emits native evidence. */
export async function verifyProvenanceInputs(inputs) {
  for (const [path, expected, cap] of inputs) {
    if (
      !Number.isInteger(cap) ||
      cap < 1 ||
      cap > 67108864 ||
      typeof expected !== "string" ||
      !/^[0-9a-f]{64}$/.test(expected)
    ) {
      throw new Error("Invalid provenance input limits");
    }
    if (digest(await boundedRead(path, cap)) !== expected)
      throw new Error("Build/input digest mismatch");
  }
}

/** Hash correlation is reproducibility metadata, not authenticity or lifetime authority. */
export async function runRecordedAcquisition(buildDirectory, scenario) {
  if (process.platform !== "win32" || process.arch !== "x64")
    throw new Error("Requires Windows x64");
  const manifestPath = join(buildDirectory, "build-result.json");
  const manifestBytes = await boundedRead(manifestPath, 65536);
  const build = validateBuildRecord(
    JSON.parse(manifestBytes.toString("utf8").replace(/^\uFEFF/, "")),
  );
  const inputs = [
    [join(buildDirectory, "controlled-fixture.exe"), build.fixture_binary_sha256, 67108864],
    [join(buildDirectory, "acquisition-probe.exe"), build.probe_binary_sha256, 67108864],
    [join(nativeDirectory, "controlled-fixture.cpp"), build.fixture_source_sha256, 1048576],
    [join(nativeDirectory, "acquisition-probe.cpp"), build.probe_source_sha256, 1048576],
    [join(nativeDirectory, "build.ps1"), build.script_sha256, 1048576],
  ];
  const verify = async () => {
    await verifyProvenanceInputs(inputs);
    if (digest(await boundedRead(manifestPath, 65536)) !== digest(manifestBytes))
      throw new Error("Build manifest changed");
  };
  await verify();
  const supervisorHash = digest(
    await boundedRead(join(nativeDirectory, "supervisor.mjs"), 1048576),
  );
  const recorderHash = digest(await boundedRead(fileURLToPath(import.meta.url), 1048576));
  const observation = await superviseAcquisition(inputs[0][0], inputs[1][0], { scenario });
  await verify();
  if (
    supervisorHash !==
      digest(await boundedRead(join(nativeDirectory, "supervisor.mjs"), 1048576)) ||
    recorderHash !== digest(await boundedRead(fileURLToPath(import.meta.url), 1048576))
  )
    throw new Error("Runner source changed");
  return {
    version: 1,
    scope: "correlated-acquisition-record",
    product_admission_granted: false,
    build,
    build_record_sha256: digest(manifestBytes),
    supervisor_sha256: supervisorHash,
    recorder_sha256: recorderHash,
    runtime: { os_release: release(), architecture: process.arch, node: process.version },
    provider: { status: "not-collected" },
    notification_order: { status: "not-collected" },
    deadline_ms: 35000,
    observation_sha256: digest(JSON.stringify(observation)),
    observation,
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 4)
      throw new Error("Usage: node run-record.mjs <build-directory> <scenario>");
    console.log(
      JSON.stringify(await runRecordedAcquisition(resolve(process.argv[2]), process.argv[3])),
    );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
