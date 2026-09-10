import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const COMMANDS = [
  ["create-a", "created"],
  ["before-root", "before-root"],
  ["before-capture", "before-capture"],
  ["replace-b", "created"],
  [null, "reuse"],
  ["before-probe", "before-probe"],
  ["before-commit", "before-commit"],
  ["stop", "stopped"],
];

/** Supervises only its own direct child. This is fixture protocol evidence, not acquisition. */
export function superviseFixture(
  executable,
  args = [],
  { deadlineMs = 35000, byteCap = 65536 } = {},
) {
  if (
    !Number.isInteger(deadlineMs) ||
    deadlineMs < 1 ||
    deadlineMs > 60000 ||
    !Number.isInteger(byteCap) ||
    byteCap < 1 ||
    byteCap > 65536
  ) {
    return Promise.reject(new Error("Invalid supervisor limits"));
  }
  return new Promise((resolveResult, reject) => {
    const child = spawn(executable, args, { stdio: ["pipe", "pipe", "pipe"], shell: false });
    let failure;
    let pending = "";
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let index = -1;
    let lastTime = -1;
    let pid;
    const observations = [];
    const fail = (message) => {
      failure ??= new Error(message);
      child.kill("SIGKILL");
    };
    const timer = setTimeout(() => fail("Fixture deadline exceeded"), deadlineMs);
    const advance = () => {
      index++;
      const command = COMMANDS[index]?.[0];
      if (command) child.stdin.write(`${command}\n`);
      if (index === COMMANDS.length) child.stdin.end();
    };
    child.stdin.on("error", (error) => fail(`Fixture stdin: ${error.message}`));
    child.on("error", (error) => {
      failure ??= error;
    });
    child.stdout.on("data", (chunk) => {
      stdoutBytes += chunk.length;
      if (stdoutBytes > byteCap) {
        fail("Fixture stdout exceeds cap");
        return;
      }
      pending += chunk.toString("utf8");
      for (;;) {
        const newline = pending.indexOf("\n");
        if (newline < 0 || failure) break;
        const line = pending.slice(0, newline);
        pending = pending.slice(newline + 1);
        try {
          const record = JSON.parse(line);
          const expected = index < 0 ? "ready" : COMMANDS[index]?.[1];
          const keys = [
            "version",
            "scope",
            "step",
            "elapsed_ms",
            "event",
            "marker",
            "handle",
            "pid",
            "win32_error",
            "product_admission_granted",
          ].sort();
          if (
            !record ||
            Object.keys(record).sort().join() !== keys.join() ||
            record.version !== 1 ||
            record.scope !== "controlled-fixture-observation" ||
            record.product_admission_granted !== false ||
            record.win32_error !== 0 ||
            record.step !== observations.length ||
            !Number.isSafeInteger(record.elapsed_ms) ||
            record.elapsed_ms < 0 ||
            record.elapsed_ms < lastTime ||
            record.elapsed_ms > 60000 ||
            !Number.isInteger(record.pid) ||
            record.pid !== child.pid ||
            (pid !== undefined && pid !== record.pid) ||
            typeof record.handle !== "string" ||
            !/^0x[0-9a-f]{1,16}$/.test(record.handle) ||
            (expected === "reuse"
              ? !["handle_reused", "handle_not_reused"].includes(record.event)
              : record.event !== expected)
          ) {
            throw new Error("Invalid or unexpected fixture observation");
          }
          const marker = index < 0 || index === 7 ? "none" : index < 3 ? "A" : "B";
          if (record.marker !== marker || (marker === "none") !== (BigInt(record.handle) === 0n)) {
            throw new Error("Inconsistent fixture state");
          }
          if (
            (index === 1 || index === 2 || index === 4 || index === 5 || index === 6) &&
            record.handle !== observations.at(-1).handle
          ) {
            throw new Error("Handle changed outside replacement");
          }
          if (
            index === 4 &&
            (record.event === "handle_reused") !==
              (BigInt(record.handle) === BigInt(observations[1].handle))
          ) {
            throw new Error("Inconsistent reuse observation");
          }
          pid = record.pid;
          lastTime = record.elapsed_ms;
          observations.push(record);
          advance();
        } catch (error) {
          fail(error.message);
        }
      }
    });
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length;
      if (stderrBytes > byteCap) fail("Fixture stderr exceeds cap");
    });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      if (failure) reject(failure);
      else if (code !== 0 || signal || pending.length || index !== COMMANDS.length || stderrBytes) {
        reject(new Error("Failed or incomplete fixture transcript"));
      } else
        resolveResult({
          version: 1,
          scope: "supervised-fixture-only",
          native_acquisition_executed: false,
          product_admission_granted: false,
          observations,
        });
    });
  });
}

/** One retained-root probe process across all scenario barriers. */
export async function superviseAcquisition(
  fixture,
  probe,
  {
    scenario = "ordinary",
    deadlineMs = 35000,
    byteCap = 65536,
    fixtureArgs = [],
    probePrefixArgs = [],
    lifecycle = {},
    abortOnReplacement = false,
  } = {},
) {
  if (
    ![
      "ordinary",
      "mismatched-pid",
      "stop-before-root",
      "stop-before-capture",
      "stop-before-probe",
      "stop-before-commit",
      "replace-before-root",
      "replace-before-capture",
      "replace-before-probe",
      "replace-before-commit",
    ].includes(scenario)
  ) {
    throw new Error("Unknown scenario");
  }
  const result = {
    version: 1,
    scope: "supervised-acquisition-experiment",
    children: [],
    scenario,
    product_admission_granted: false,
  };
  if (
    !Number.isInteger(deadlineMs) ||
    deadlineMs < 1 ||
    deadlineMs > 60000 ||
    !Number.isInteger(byteCap) ||
    byteCap < 1 ||
    byteCap > 65536
  )
    throw new Error("Invalid supervisor limits");
  const children = [];
  let failure;
  const killAll = () => {
    for (const entry of children) if (!entry.closed) entry.child.kill("SIGKILL");
  };
  const fail = (message, reason = "protocol_failed") => {
    lifecycle.terminal?.(reason);
    failure ??= new Error(message);
    killAll();
  };
  const timer = setTimeout(() => fail("Acquisition deadline exceeded", "timed_out"), deadlineMs);
  function launch(executable, args) {
    const child = spawn(executable, args, { stdio: ["pipe", "pipe", "pipe"], shell: false });
    const entry = {
      child,
      closed: false,
      stdout: "",
      rawStdout: "",
      rawStderr: "",
      stderr: 0,
      bytes: 0,
      code: null,
    };
    children.push(entry);
    entry.done = new Promise((done) => {
      child.on("error", (error) => fail(error.message, "native_failed"));
      child.on("close", (code, signal) => {
        entry.closed = true;
        entry.code = code;
        entry.signal = signal;
        done();
      });
    });
    child.stdin.on("error", (error) => fail(error.message));
    child.stdout.on("data", (chunk) => {
      entry.bytes += chunk.length;
      entry.rawStdout = (entry.rawStdout + chunk.toString("utf8")).slice(0, byteCap);
      if (entry.bytes > byteCap) fail("Child stdout cap exceeded");
      else entry.stdout += chunk.toString("utf8");
    });
    child.stderr.on("data", (chunk) => {
      entry.stderr += chunk.length;
      entry.rawStderr = (entry.rawStderr + chunk.toString("utf8")).slice(0, byteCap);
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
  const observations = [];
  let lastTime = -1;
  try {
    if (lifecycle.cancelled?.()) {
      lifecycle.terminal?.("cancelled");
      throw new Error("Cancelled before native startup");
    }
    lifecycle.begin?.();
    const owner = launch(fixture, fixtureArgs);
    async function receive(event, marker) {
      await until(() => owner.stdout.includes("\n") || owner.closed);
      const end = owner.stdout.indexOf("\n");
      if (end < 0) throw new Error("Incomplete fixture transcript");
      const record = JSON.parse(owner.stdout.slice(0, end));
      owner.stdout = owner.stdout.slice(end + 1);
      if (
        record.version !== 1 ||
        record.scope !== "controlled-fixture-observation" ||
        record.pid !== owner.child.pid ||
        record.step !== observations.length ||
        !Number.isSafeInteger(record.elapsed_ms) ||
        record.elapsed_ms < 0 ||
        record.elapsed_ms < lastTime ||
        record.elapsed_ms > 60000 ||
        record.event !== event ||
        record.marker !== marker ||
        record.win32_error !== 0 ||
        record.product_admission_granted !== false ||
        typeof record.handle !== "string" ||
        !/^0x[0-9a-f]{1,16}$/.test(record.handle) ||
        (marker === "none") !== (BigInt(record.handle) === 0n)
      )
        throw new Error("Invalid fixture observation");
      observations.push(record);
      lastTime = record.elapsed_ms;
      return record;
    }
    await receive("ready", "none");
    owner.child.stdin.write("create-a\n");
    let current = await receive("created", "A");
    let reuse = false;
    async function replace() {
      lifecycle.terminal?.("authority_uncertain");
      owner.child.stdin.write("replace-b\n");
      const previous = current;
      current = await receive("created", "B");
      reuse = BigInt(previous.handle) === BigInt(current.handle);
      const reuseEvent = await receive(reuse ? "handle_reused" : "handle_not_reused", "B");
      if (reuseEvent.handle !== current.handle) throw new Error("Reuse event changed B handle");
    }
    owner.child.stdin.write("before-root\n");
    const barrier = await receive("before-root", current.marker);
    if (barrier.handle !== current.handle)
      throw new Error("Fixture handle changed outside replacement");
    // A live supervisor cannot share its direct child's PID. Keep the HWND owned
    // by our fixture; this negative control never targets another process's window.
    const expectedPid = scenario === "mismatched-pid" ? process.pid : owner.child.pid;
    const reader = launch(probe, [...probePrefixArgs, current.handle, String(expectedPid)]);
    const barriers = ["before-root", "before-capture", "before-probe", "before-commit"];
    let finalLine;
    let stopped = false;
    let authorityAborted = false;
    let completedBarriers = 0;
    for (const name of barriers) {
      await until(() => reader.stdout.includes("\n") || reader.closed || owner.closed);
      if (owner.closed) throw new Error("Fixture exited during acquisition");
      const end = reader.stdout.indexOf("\n");
      if (end < 0) throw new Error("Missing probe barrier");
      const line = reader.stdout.slice(0, end);
      reader.stdout = reader.stdout.slice(end + 1);
      const message = JSON.parse(line);
      if (message.scope === "isolated-acquisition-observation") {
        finalLine = line;
        break;
      }
      if (
        message.version !== 1 ||
        message.scope !== "acquisition-barrier" ||
        message.barrier !== name ||
        Object.keys(message).sort().join() !== "barrier,scope,version"
      )
        throw new Error("Invalid probe barrier");
      if (scenario === `replace-${name}`) {
        await replace();
        if (abortOnReplacement) {
          reader.child.stdin.write("abort\n");
          authorityAborted = true;
          break;
        }
      }
      if (scenario === `stop-${name}`) {
        lifecycle.terminal?.("cancelled");
        owner.child.stdin.write("stop\n");
        await receive("stopped", "none");
        owner.child.stdin.end();
        await until(() => owner.closed);
        stopped = true;
        // The probe's bounded barrier accepts only continue. Explicit abort
        // prevents its next acquisition call; already completed calls remain facts.
        reader.child.stdin.write("abort\n");
        break;
      }
      reader.child.stdin.write("continue\n");
      completedBarriers++;
    }
    reader.child.stdin.end();
    await until(() => reader.closed || (!stopped && owner.closed));
    if (!stopped && owner.closed) throw new Error("Fixture exited during acquisition");
    if (
      finalLine !== undefined
        ? reader.stdout.length !== 0
        : !reader.stdout.endsWith("\n") || reader.stdout.trim().includes("\n")
    )
      throw new Error("Invalid probe output");
    if (reader.stderr) throw new Error("Unexpected probe stderr");
    const acquisition = JSON.parse(finalLine ?? reader.stdout);
    const fullKeys = [
      "version",
      "scope",
      "stage",
      "hresult",
      "elapsed_ms",
      "root_runtime_id",
      "probe_runtime_id",
      "equal",
      "sampled",
      "width",
      "height",
      "bgra",
      "marker",
      "product_admission_granted",
    ].sort();
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
    const validId = (value) =>
      value === null ||
      (Array.isArray(value) &&
        value.length > 0 &&
        value.length <= 64 &&
        value.every((part) => Number.isInteger(part) && part >= -2147483648 && part <= 2147483647));
    if (
      !acquisition ||
      Object.keys(acquisition).sort().join() !== fullKeys.join() ||
      !stages.includes(acquisition.stage) ||
      !Number.isSafeInteger(acquisition.elapsed_ms) ||
      acquisition.elapsed_ms < 0 ||
      acquisition.elapsed_ms > 60000 ||
      !validId(acquisition.root_runtime_id) ||
      !validId(acquisition.probe_runtime_id) ||
      !(acquisition.equal === null || typeof acquisition.equal === "boolean") ||
      !Number.isInteger(acquisition.width) ||
      !Number.isInteger(acquisition.height) ||
      acquisition.width < 0 ||
      acquisition.height < 0 ||
      acquisition.width > 4096 ||
      acquisition.height > 4096 ||
      !Array.isArray(acquisition.bgra) ||
      acquisition.bgra.length !== 4 ||
      !acquisition.bgra.every((value) => Number.isInteger(value) && value >= 0 && value <= 255)
    )
      throw new Error("Invalid acquisition fields");
    if (
      acquisition.version !== 1 ||
      acquisition.scope !== "isolated-acquisition-observation" ||
      acquisition.product_admission_granted !== false ||
      !Number.isInteger(acquisition.hresult) ||
      acquisition.hresult < -2147483648 ||
      acquisition.hresult > 2147483647 ||
      typeof acquisition.sampled !== "boolean" ||
      !["A", "B", "unknown"].includes(acquisition.marker)
    )
      throw new Error("Invalid acquisition observation");
    if (acquisition.hresult >= 0 && completedBarriers !== 4)
      throw new Error("Incomplete successful probe");
    const expectedMarker =
      acquisition.sampled && acquisition.bgra.slice(0, 3).join() === "220,80,30"
        ? "A"
        : acquisition.sampled && acquisition.bgra.slice(0, 3).join() === "70,210,40"
          ? "B"
          : "unknown";
    if (
      acquisition.marker !== expectedMarker ||
      (acquisition.sampled
        ? acquisition.width < 1 || acquisition.height < 1
        : acquisition.width !== 0 ||
          acquisition.height !== 0 ||
          acquisition.bgra.some((value) => value !== 0)) ||
      (acquisition.equal !== null &&
        (acquisition.root_runtime_id === null || acquisition.probe_runtime_id === null)) ||
      (acquisition.hresult >= 0 &&
        (acquisition.stage !== "observed" ||
          acquisition.hresult !== 0 ||
          !acquisition.sampled ||
          acquisition.root_runtime_id === null ||
          acquisition.probe_runtime_id === null ||
          acquisition.equal === null)) ||
      (acquisition.hresult < 0 && acquisition.stage === "observed")
    )
      throw new Error("Inconsistent acquisition facts");
    const stageIndex = stages.indexOf(acquisition.stage);
    if (
      acquisition.stage !== "cleanup" &&
      ((stageIndex < 4 && acquisition.root_runtime_id !== null) ||
        (stageIndex > 4 && acquisition.root_runtime_id === null) ||
        (stageIndex < 10 && acquisition.sampled) ||
        (stageIndex > 10 && !acquisition.sampled) ||
        (stageIndex < 12 && acquisition.probe_runtime_id !== null) ||
        (stageIndex > 12 && acquisition.probe_runtime_id === null) ||
        (stageIndex < 13 && acquisition.equal !== null))
    )
      throw new Error("Facts precede their acquisition stage");
    if (!stopped) {
      owner.child.stdin.write("stop\n");
      await receive("stopped", "none");
      owner.child.stdin.end();
      await until(() => owner.closed);
    }
    if (owner.code !== 0 || owner.signal || owner.stderr || owner.stdout)
      throw new Error("Incomplete fixture shutdown");
    const failed = reader.code !== 0 || reader.signal || acquisition.hresult < 0;
    const stoppedStage = {
      "stop-before-root": "uia_create",
      "stop-before-capture": "uia_root_runtime_id",
      "stop-before-probe": "pixel_map",
      "stop-before-commit": "uia_compare",
    }[authorityAborted ? scenario.replace("replace-", "stop-") : scenario];
    if (
      (stopped || authorityAborted) &&
      (reader.code !== 1 ||
        acquisition.hresult !== -2147467260 ||
        acquisition.stage !== stoppedStage)
    )
      throw new Error("Stopped probe did not abort at its barrier");
    if (
      scenario === "mismatched-pid" &&
      (reader.code !== 1 || acquisition.stage !== "input" || acquisition.hresult !== -2147024891)
    )
      throw new Error("PID negative control did not reject before acquisition");
    if (
      scenario === "mismatched-pid" ||
      (!failed && (acquisition.equal !== true || acquisition.marker !== "A"))
    )
      lifecycle.terminal?.("authority_uncertain");
    else if (failed && !stopped && !authorityAborted) lifecycle.terminal?.("native_failed");
    lifecycle.result?.(acquisition);
    const unknown = !acquisition.sampled || acquisition.marker === "unknown";
    return {
      ...result,
      outcome: stopped
        ? "stopped"
        : failed
          ? "failed"
          : unknown || (scenario !== "ordinary" && !reuse)
            ? "inconclusive"
            : "observation",
      reason: stopped
        ? "Explicit barrier abort; no in-flight call cancellation claimed"
        : failed
          ? "Probe failure"
          : unknown
            ? "Marker unavailable"
            : scenario !== "ordinary" && !reuse
              ? "Handle reuse not observed"
              : "No lifetime admission inferred",
      observations,
      expected_pid: expectedPid,
      acquisition,
    };
  } catch (error) {
    lifecycle.terminal?.("protocol_failed");
    throw error;
  } finally {
    clearTimeout(timer);
    killAll();
    let cleanupTimer;
    const closed = await Promise.race([
      Promise.all(children.map((entry) => entry.done)).then(() => true),
      new Promise((done) => {
        cleanupTimer = setTimeout(() => done(false), 2000);
      }),
    ]);
    clearTimeout(cleanupTimer);
    result.children.push(
      ...children.map((entry, index) => ({
        role: index === 0 ? "fixture" : "probe",
        pid: entry.child.pid,
        closed: entry.closed,
        code: entry.code,
        signal: entry.signal ?? null,
        stdout: entry.rawStdout,
        stderr: entry.rawStderr,
        stdoutBytes: entry.bytes,
        stderrBytes: entry.stderr,
      })),
    );
    lifecycle.evidence?.(result.children);
    lifecycle.cleanup?.(closed ? "closed" : "termination-unconfirmed");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const acquisitionMode = process.argv[2] === "--acquisition" && process.argv.length === 6;
  if (
    process.platform !== "win32" ||
    process.arch !== "x64" ||
    (!acquisitionMode && process.argv.length !== 3)
  ) {
    console.error("Requires Windows x64 and one controlled-fixture executable path.");
    process.exitCode = 2;
  } else {
    try {
      console.log(
        JSON.stringify(
          acquisitionMode
            ? await superviseAcquisition(resolve(process.argv[3]), resolve(process.argv[4]), {
                scenario: process.argv[5],
              })
            : await superviseFixture(resolve(process.argv[2])),
        ),
      );
    } catch (error) {
      console.error(error.message);
      process.exitCode = 1;
    }
  }
}
