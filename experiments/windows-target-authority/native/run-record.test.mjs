import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  validateBuildRecord,
  runRecordedAcquisition,
  verifyProvenanceInputs,
} from "./run-record.mjs";

const build = {
  version: 3,
  scope: "sdk-smoke-fixture-and-acquisition-probe-build-only",
  source_sha: "a".repeat(40),
  worktree_dirty: true,
  fixture_executed: false,
  probe_executed: false,
  executable: {
    architecture: "x64",
    native_acquisition_executed: false,
    product_admission_granted: false,
  },
  ...Object.fromEntries(
    [
      "fixture_source_sha256",
      "fixture_binary_sha256",
      "probe_source_sha256",
      "probe_binary_sha256",
      "script_sha256",
      "compiler_sha256",
    ].map((key) => [key, "b".repeat(64)]),
  ),
  sdk_version: "synthetic",
  toolset_version: "synthetic",
  os_version: "synthetic",
  os_build: "synthetic",
};
test("synthetic build record retains dirty-source and unexecuted status", () => {
  assert.equal(validateBuildRecord(build), build);
});
for (const [key, value] of [
  ["version", 2],
  ["fixture_executed", true],
  ["source_sha", "HEAD"],
  ["source_sha", ["a".repeat(40)]],
  ["probe_binary_sha256", "unknown"],
  ["sdk_version", ""],
]) {
  test(`rejects invalid provenance ${key}`, () =>
    assert.throws(() => validateBuildRecord({ ...build, [key]: value })));
}
test("recorded native execution cannot silently run on macOS", async () => {
  if (process.platform !== "win32" || process.arch !== "x64")
    await assert.rejects(runRecordedAcquisition("unused", "ordinary"), /Requires Windows x64/);
});

test("portable synthetic files enforce binary/source hashes and post-run changes", async () => {
  const directory = await mkdtemp(join(tmpdir(), "lens-provenance-test-"));
  try {
    const binary = join(directory, "synthetic.bin");
    const source = join(directory, "synthetic.cpp");
    const bytes = Buffer.from("synthetic; not an executable or native observation");
    const hash = createHash("sha256").update(bytes).digest("hex");
    await writeFile(binary, bytes);
    await writeFile(source, bytes);
    const inputs = [
      [binary, hash, 128],
      [source, hash, 128],
    ];
    await verifyProvenanceInputs(inputs);
    await writeFile(binary, "changed");
    await assert.rejects(verifyProvenanceInputs(inputs), /digest mismatch/);
    await writeFile(binary, bytes);
    await writeFile(source, "changed");
    await assert.rejects(verifyProvenanceInputs(inputs), /digest mismatch/);
    await writeFile(source, bytes);
    await verifyProvenanceInputs(inputs);
    await assert.rejects(verifyProvenanceInputs([[binary, hash, 2]]), /exceeds bounds/);
    await assert.rejects(verifyProvenanceInputs([[directory, hash, 128]]), /exceeds bounds/);
    await assert.rejects(verifyProvenanceInputs([[binary, hash, -1]]), /input limits/);
  } finally {
    await rm(directory, { recursive: true });
  }
});
