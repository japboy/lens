import { randomUUID, createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { release } from "node:os";
import { pathToFileURL } from "node:url";
import { createOperation } from "./state.mjs";
import { supervise as superviseProvider } from "../native/provider-stall/supervisor.mjs";
import { superviseAcquisition } from "../native/supervisor.mjs";

/** Native records are bound to the owned one-shot channel, not a native receipt echo.
 * Only fixed fixture/probe binaries without descendants are supported. No shell or
 * supervisor subprocess is spawned: native supervisors own every direct child.
 */
export async function runOperation({
  kind,
  fixture,
  probe,
  scenario,
  operation = randomUUID(),
  receipt = randomUUID(),
  readSequence = "1",
  deadlineMs = 10000,
  stallMs = 1000,
  fixtureArgs = [],
  probePrefixArgs = [],
  cancelBeforeStart = false,
  spawnChild,
}) {
  if (!["provider", "acquisition"].includes(kind)) throw new Error("Invalid experiment kind");
  const state = createOperation({ operation, receipt, readSequence });
  const transitions = [];
  const started = performance.now();
  let payload;
  let cleanup = "pending";
  let children = [];
  const record = (event) =>
    transitions.push({ event, elapsedMs: performance.now() - started, state: state.snapshot() });
  const lifecycle = {
    cancelled: () => cancelBeforeStart,
    begin() {
      state.begin();
      record("begin");
    },
    terminal(reason) {
      if (state.finish(reason)) record("terminal");
    },
    result(value) {
      if (state.accept({ operation, receipt, readSequence, payload: value })) payload = value;
      record("result");
    },
    cleanup(value) {
      cleanup = value;
    },
    evidence(value) {
      children = value;
    },
  };
  let native;
  let error;
  try {
    if (cancelBeforeStart) {
      lifecycle.terminal("cancelled");
      cleanup = "closed";
      native = { children: [], cleanup: "closed", outcome: "stopped" };
    } else {
      const supervise = kind === "provider" ? superviseProvider : superviseAcquisition;
      native = await supervise(fixture, probe, {
        scenario,
        deadlineMs,
        stallMs,
        fixtureArgs,
        probePrefixArgs,
        lifecycle,
        abortOnReplacement: true,
        spawnChild,
      });
      cleanup = native.cleanup ?? cleanup;
      if (state.snapshot().stage !== "terminal") {
        lifecycle.terminal(
          native.outcome === "observation" && state.snapshot().accepted === 1
            ? "completed_observation"
            : "native_failed",
        );
      }
    }
  } catch (cause) {
    error = cause.message;
    lifecycle.terminal("native_failed");
  }
  // Only explicit close observations count, including zero children on pre-start Stop.
  state.cleanup(cleanup === "closed" ? "closed" : "termination-unconfirmed");
  record("cleanup");
  const snapshot = state.snapshot();
  const verified =
    !error &&
    snapshot.cleanup === "closed" &&
    ["completed_observation", "cancelled", "timed_out", "authority_uncertain"].includes(
      snapshot.reason,
    ) &&
    (kind === "acquisition" || native?.outcome === "observation" || cancelBeforeStart);
  return {
    version: 1,
    scope: "isolated-owned-fixture-adapter",
    kind,
    scenario,
    ...snapshot,
    elapsedMs: performance.now() - started,
    transitions,
    verification: verified ? "observed" : "failed",
    published:
      snapshot.reason === "completed_observation" && snapshot.cleanup === "closed" ? payload : null,
    native: native ?? { children },
    ...(error ? { error } : {}),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.platform !== "win32" || process.arch !== "x64" || process.argv.length !== 6) {
    throw new Error("Windows x64: runner.mjs provider|acquisition fixture.exe probe.exe scenario");
  }
  const hash = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
  const paths = [
    process.argv[1],
    new URL("./state.mjs", import.meta.url),
    new URL("../native/supervisor.mjs", import.meta.url),
    new URL("../native/provider-stall/supervisor.mjs", import.meta.url),
    process.argv[3],
    process.argv[4],
  ];
  const hashes = paths.map(hash);
  const result = await runOperation({
    kind: process.argv[2],
    fixture: process.argv[3],
    probe: process.argv[4],
    scenario: process.argv[5],
  });
  result.provenance = {
    sha256: hashes,
    osRelease: release(),
    architecture: process.arch,
    nodeVersion: process.version,
    sourceFiles: paths.map(String),
  };
  let unchanged = false;
  try {
    unchanged = paths.every((path, index) => hash(path) === hashes[index]);
  } catch {
    // Missing/changed input after execution invalidates publication as well as exit status.
  }
  if (!unchanged) {
    result.published = null;
    result.verification = "failed";
    result.provenanceFailure = "Run inputs changed or could not be rechecked";
  }
  result.inputsUnchanged = unchanged;
  console.log(JSON.stringify(result));
  process.exitCode = result.verification === "observed" ? 0 : 1;
}
