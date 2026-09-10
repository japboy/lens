import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const fixtureKeys =
  "elapsed_ms,event,handle,marker,pid,product_admission_granted,scope,step,version,win32_error";
const resultKeys =
  "bgra,elapsed_ms,equal,height,hresult,marker,probe_runtime_id,product_admission_granted,root_runtime_id,sampled,scope,stage,version,width";
const barriers = ["before-root", "before-capture", "before-probe", "before-commit"];
const stages = [
  "input",
  "mta",
  "uia_create",
  "uia_root",
  "uia_root_runtime_id",
  "capture_support",
  "capture_item",
  "d3d_device",
  "capture_start",
  "capture_frame",
  "pixel_map",
  "uia_probe",
  "uia_probe_runtime_id",
  "uia_compare",
  "observed",
  "cleanup",
];
function ensure(value, message) {
  if (!value) throw new Error(message);
}
function validateAcquisition(value, completed) {
  const id = (v) =>
    v === null ||
    (Array.isArray(v) &&
      v.length > 0 &&
      v.length <= 64 &&
      v.every((n) => Number.isInteger(n) && n >= -2147483648 && n <= 2147483647));
  ensure(
    value &&
      Object.keys(value).sort().join() === resultKeys &&
      value.version === 1 &&
      value.scope === "isolated-acquisition-observation" &&
      value.product_admission_granted === false,
    "Invalid acquisition schema",
  );
  ensure(
    stages.includes(value.stage) &&
      Number.isInteger(value.hresult) &&
      value.hresult >= -2147483648 &&
      value.hresult <= 2147483647 &&
      Number.isSafeInteger(value.elapsed_ms) &&
      value.elapsed_ms >= 0 &&
      value.elapsed_ms <= 60000,
    "Invalid acquisition status",
  );
  ensure(
    id(value.root_runtime_id) &&
      id(value.probe_runtime_id) &&
      (value.equal === null || typeof value.equal === "boolean") &&
      typeof value.sampled === "boolean",
    "Invalid acquisition facts",
  );
  ensure(
    [value.width, value.height].every((n) => Number.isInteger(n) && n >= 0 && n <= 4096) &&
      Array.isArray(value.bgra) &&
      value.bgra.length === 4 &&
      value.bgra.every((n) => Number.isInteger(n) && n >= 0 && n <= 255),
    "Invalid pixel facts",
  );
  const marker =
    value.sampled && value.bgra.slice(0, 3).join() === "220,80,30"
      ? "A"
      : value.sampled && value.bgra.slice(0, 3).join() === "70,210,40"
        ? "B"
        : "unknown";
  ensure(
    value.marker === marker &&
      (value.sampled
        ? value.width > 0 && value.height > 0
        : value.width === 0 && value.height === 0 && value.bgra.every((n) => n === 0)),
    "Inconsistent pixel facts",
  );
  ensure(
    value.equal === null || (value.root_runtime_id !== null && value.probe_runtime_id !== null),
    "Comparison lacks IDs",
  );
  const stage = stages.indexOf(value.stage);
  ensure(
    value.stage === "cleanup" ||
      !(
        (stage < 4 && value.root_runtime_id !== null) ||
        (stage > 4 && value.root_runtime_id === null) ||
        (stage < 10 && value.sampled) ||
        (stage > 10 && !value.sampled) ||
        (stage < 12 && value.probe_runtime_id !== null) ||
        (stage > 12 && value.probe_runtime_id === null) ||
        (stage < 13 && value.equal !== null)
      ),
    "Facts precede stage",
  );
  ensure(
    value.hresult < 0
      ? value.stage !== "observed"
      : value.hresult === 0 &&
          value.stage === "observed" &&
          completed === 4 &&
          value.sampled &&
          value.root_runtime_id !== null &&
          value.probe_runtime_id !== null &&
          value.equal !== null,
    "Invalid success facts",
  );
  return value;
}

/** Three owned direct children only. No new HWND is ever sent to the retained reader. */
export async function superviseCrossProcess(
  fixture,
  probe,
  {
    scenario = "replace-before-probe",
    deadlineMs = 35000,
    byteCap = 65536,
    fixtureArgs = [],
    replacementArgs = fixtureArgs,
    probePrefixArgs = [],
  } = {},
) {
  ensure(
    ["replace-before-probe", "replace-before-commit"].includes(scenario),
    "Unknown cross-process scenario",
  );
  ensure(
    Number.isInteger(deadlineMs) &&
      deadlineMs > 0 &&
      deadlineMs <= 60000 &&
      Number.isInteger(byteCap) &&
      byteCap > 0 &&
      byteCap <= 65536,
    "Invalid limits",
  );
  const children = [];
  let failure;
  const kill = () => {
    for (const entry of children) if (!entry.closed) entry.child.kill("SIGKILL");
  };
  const fail = (message) => {
    failure ??= new Error(message);
    kill();
  };
  const timer = setTimeout(() => fail("Cross-process deadline exceeded"), deadlineMs);
  function launch(executable, args) {
    const child = spawn(executable, args, { shell: false, stdio: ["pipe", "pipe", "pipe"] });
    const entry = {
      child,
      closed: false,
      text: "",
      bytes: 0,
      stderr: 0,
      records: [],
      lastTime: -1,
    };
    children.push(entry);
    entry.done = new Promise((done) => {
      child.on("error", (error) => fail(error.message));
      child.on("close", (code, signal) => {
        Object.assign(entry, { closed: true, code, signal });
        done();
      });
    });
    child.stdin.on("error", (error) => fail(error.message));
    child.stdout.on("data", (chunk) => {
      entry.bytes += chunk.length;
      if (entry.bytes > byteCap) fail("Child stdout cap exceeded");
      else entry.text += chunk.toString("utf8");
    });
    child.stderr.on("data", (chunk) => {
      entry.stderr += chunk.length;
      if (entry.stderr > byteCap) fail("Child stderr cap exceeded");
    });
    return entry;
  }
  async function until(predicate) {
    while (!predicate()) {
      if (failure) throw failure;
      await new Promise((done) => setTimeout(done, 5));
    }
    if (failure) throw failure;
  }
  async function line(entry) {
    await until(() => entry.text.includes("\n") || entry.closed);
    const end = entry.text.indexOf("\n");
    ensure(end >= 0, "Incomplete child transcript");
    const value = JSON.parse(entry.text.slice(0, end));
    entry.text = entry.text.slice(end + 1);
    return value;
  }
  async function receive(entry, event, marker) {
    const value = await line(entry);
    ensure(
      value &&
        Object.keys(value).sort().join() === fixtureKeys &&
        value.version === 1 &&
        value.scope === "controlled-fixture-observation" &&
        value.pid === entry.child.pid &&
        value.step === entry.records.length &&
        value.event === event &&
        value.marker === marker &&
        value.win32_error === 0 &&
        value.product_admission_granted === false &&
        typeof value.handle === "string" &&
        /^0x[0-9a-f]{1,16}$/.test(value.handle) &&
        (marker === "none") === (BigInt(value.handle) === 0n),
      "Invalid fixture record",
    );
    ensure(
      Number.isSafeInteger(value.elapsed_ms) &&
        value.elapsed_ms >= 0 &&
        value.elapsed_ms >= entry.lastTime &&
        value.elapsed_ms <= 60000,
      "Invalid fixture time",
    );
    entry.lastTime = value.elapsed_ms;
    entry.records.push(value);
    return value;
  }
  async function stop(entry) {
    entry.child.stdin.write("stop\n");
    await receive(entry, "stopped", "none");
    entry.child.stdin.end();
    await until(() => entry.closed);
    ensure(
      entry.code === 0 && !entry.signal && !entry.stderr && !entry.text,
      "Invalid fixture shutdown",
    );
  }
  try {
    const a = launch(fixture, fixtureArgs);
    await receive(a, "ready", "none");
    a.child.stdin.write("create-a\n");
    const original = await receive(a, "created", "A");
    const reader = launch(probe, [...probePrefixArgs, original.handle, String(a.child.pid)]);
    let b,
      replacement,
      final,
      abort = false,
      completed = 0;
    for (const barrier of barriers) {
      const message = await line(reader);
      if (message?.scope === "isolated-acquisition-observation") {
        final = message;
        break;
      }
      ensure(
        message &&
          Object.keys(message).sort().join() === "barrier,scope,version" &&
          message.version === 1 &&
          message.scope === "acquisition-barrier" &&
          message.barrier === barrier,
        "Invalid probe barrier",
      );
      ensure(!(b ?? a).closed, "Fixture exited during acquisition");
      if (scenario === `replace-${barrier}`) {
        await stop(a);
        b = launch(fixture, replacementArgs);
        await receive(b, "ready", "none");
        b.child.stdin.write("create-a\n");
        const first = await receive(b, "created", "A");
        b.child.stdin.write("replace-b\n");
        replacement = await receive(b, "created", "B");
        const reuse = await receive(
          b,
          BigInt(first.handle) === BigInt(replacement.handle)
            ? "handle_reused"
            : "handle_not_reused",
          "B",
        );
        ensure(reuse.handle === replacement.handle, "Replacement handle mismatch");
        // Confirm the live owned B window immediately before releasing the reader.
        b.child.stdin.write(`${barrier}\n`);
        const ack = await receive(b, barrier, "B");
        ensure(ack.handle === replacement.handle && !b.closed, "Replacement ownership unavailable");
        abort =
          b.child.pid === a.child.pid || BigInt(replacement.handle) !== BigInt(original.handle);
        if (abort) {
          reader.child.stdin.write("abort\n");
          break;
        }
      }
      reader.child.stdin.write("continue\n");
      completed++;
    }
    reader.child.stdin.end();
    if (!final) final = await line(reader);
    await until(() => reader.closed);
    ensure(!reader.text && !reader.stderr && !reader.signal, "Invalid reader shutdown");
    const acquisition = validateAcquisition(final, completed);
    ensure(reader.code === (acquisition.hresult < 0 ? 1 : 0), "Probe exit status mismatch");
    if (abort)
      ensure(
        acquisition.hresult === -2147467260 &&
          acquisition.stage === (scenario === "replace-before-probe" ? "pixel_map" : "uia_compare"),
        "Unsafe continuation after missing owned reuse",
      );
    ensure(!(b ?? a).closed, "Fixture exited before result");
    await stop(b ?? a);
    return {
      version: 1,
      scope: "cross-process-acquisition-observation",
      scenario,
      product_admission_granted: false,
      outcome: abort
        ? "inconclusive"
        : acquisition.hresult < 0
          ? "failed"
          : acquisition.marker === "unknown"
            ? "inconclusive"
            : "observation",
      reason: abort
        ? "Distinct live owned replacement PID/old HWND reuse not observed; reader aborted"
        : "Sequential observations only; no lifetime or in-flight cancellation guarantee",
      replacement_performed: !!b,
      owned_handle_reuse: !!b && !abort,
      original: { pid: a.child.pid, handle: original.handle },
      replacement: b ? { pid: b.child.pid, handle: replacement.handle } : null,
      fixture_observations: { original: a.records, replacement: b?.records ?? [] },
      acquisition,
    };
  } finally {
    clearTimeout(timer);
    kill();
    await Promise.all(children.map((entry) => entry.done));
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    ensure(process.platform === "win32" && process.arch === "x64", "Requires Windows x64");
    ensure(
      process.argv.length === 5,
      "Usage: node cross-process-supervisor.mjs <fixture.exe> <probe.exe> <scenario>",
    );
    console.log(
      JSON.stringify(
        await superviseCrossProcess(resolve(process.argv[2]), resolve(process.argv[3]), {
          scenario: process.argv[4],
        }),
      ),
    );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
