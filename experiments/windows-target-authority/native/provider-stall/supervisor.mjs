import { spawn } from "node:child_process";
import { pathToFileURL } from "node:url";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { release } from "node:os";

export function parseRecord(line, role, pid, sequence) {
  const value = JSON.parse(line);
  const keys = [
    "event",
    "hresult",
    "pid",
    "seq",
    "version",
    ...(role === "probe" ? ["marker"] : []),
  ].sort();
  const events =
    role === "fixture"
      ? [
          "ready",
          "armed",
          "provider_entered",
          "released",
          "provider_returned",
          "stopping",
          "closed",
          "protocol_failed",
        ]
      : ["root_ready", "call_started", "call_returned", "failed"];
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    JSON.stringify(Object.keys(value).sort()) !== JSON.stringify(keys) ||
    value.version !== 1 ||
    value.pid !== pid ||
    value.seq !== sequence ||
    !events.includes(value.event) ||
    !Number.isInteger(value.hresult) ||
    value.hresult < -2147483648 ||
    value.hresult > 2147483647 ||
    (role === "probe" && typeof value.marker !== "boolean")
  )
    throw new Error("Invalid native record");
  if (
    !["provider_returned", "call_returned", "failed", "protocol_failed"].includes(value.event) &&
    value.hresult !== 0
  )
    throw new Error("Nonterminal event failed");
  if (role === "probe" && value.marker && (value.event !== "call_returned" || value.hresult !== 0))
    throw new Error("Impossible marker");
  return value;
}

export async function supervise(
  fixture,
  probe,
  {
    scenario = "release",
    fixtureArgs = [],
    probePrefixArgs = [],
    deadlineMs = 10000,
    stallMs = 1000,
  } = {},
) {
  if (
    !["release", "provider-deadline", "stop-inflight"].includes(scenario) ||
    !Number.isInteger(deadlineMs) ||
    deadlineMs < 100 ||
    deadlineMs > 60000 ||
    !Number.isInteger(stallMs) ||
    stallMs < 1 ||
    stallMs >= deadlineMs
  )
    throw new Error("Invalid scenario bounds");
  const children = [];
  const transcript = [];
  let terminal = false;
  let admitted = 0;
  let rejectedLate = 0;
  let failure;
  let notify = () => {};
  const signal = () => notify();
  const fail = (message) => {
    failure ??= new Error(message);
    for (const item of children) if (!item.closed) item.process.kill("SIGKILL");
    signal();
  };
  const timer = setTimeout(() => fail("External experiment deadline"), deadlineMs);
  function launch(command, args, role) {
    const process = spawn(command, args, { shell: false, stdio: ["pipe", "pipe", "pipe"] });
    const item = {
      process,
      role,
      records: [],
      cursor: 0,
      closed: false,
      code: null,
      bytes: 0,
      remainder: "",
    };
    children.push(item);
    item.done = new Promise((resolve) =>
      process.once("close", (code) => {
        item.closed = true;
        item.code = code;
        if (item.remainder) fail("Incomplete stdout line");
        resolve();
        signal();
      }),
    );
    process.on("error", (error) => fail(`Child failure: ${error.code ?? "unknown"}`));
    process.stdin.on("error", () => fail("Child command pipe failed"));
    process.stderr.on("data", () => fail("Unexpected child stderr"));
    process.stdout.setEncoding("utf8");
    process.stdout.on("data", (chunk) => {
      item.bytes += Buffer.byteLength(chunk);
      if (item.bytes > 65536) {
        fail("Child stdout limit");
        return;
      }
      item.remainder += chunk;
      while (item.remainder.includes("\n")) {
        const end = item.remainder.indexOf("\n");
        const line = item.remainder.slice(0, end).replace(/\r$/, "");
        item.remainder = item.remainder.slice(end + 1);
        try {
          if (line.length > 1024 || item.records.length >= 16) throw new Error("Record limit");
          const record = parseRecord(line, role, process.pid, item.records.length + 1);
          item.records.push(record);
          transcript.push({ role, ...record });
        } catch {
          fail("Invalid or excessive child record");
        }
      }
      if (item.remainder.length > 1024) fail("Line limit");
      signal();
    });
    return item;
  }
  async function waitUntil(predicate) {
    while (!predicate()) {
      if (failure) throw failure;
      await new Promise((resolve) => {
        notify = resolve;
      });
    }
    if (failure) throw failure;
  }
  async function next(item, event) {
    await waitUntil(() => item.records.length > item.cursor || item.closed);
    const record = item.records[item.cursor++];
    if (!record || record.event !== event) throw new Error(`Expected ${item.role}:${event}`);
    return record;
  }
  function send(item, command) {
    if (failure || item.closed) throw failure ?? new Error("Child already closed");
    item.process.stdin.write(`${command}\n`);
  }
  const result = {
    version: 1,
    scope: "controlled-provider-stall-observation",
    scenario,
    outcome: "failed",
    provider_entered: false,
    admitted_results: 0,
    rejected_late_results: 0,
    cleanup: "pending",
    product_admission: false,
    transcript,
  };
  try {
    const provider = launch(fixture, fixtureArgs, "fixture");
    await next(provider, "ready");
    const client = launch(probe, [...probePrefixArgs, String(provider.process.pid)], "probe");
    await next(client, "root_ready");
    send(provider, "arm");
    await next(provider, "armed");
    send(client, "go");
    await next(client, "call_started");
    await next(provider, "provider_entered");
    result.provider_entered = true;
    if (scenario === "provider-deadline") {
      let elapsed = false;
      const holdTimer = setTimeout(() => {
        elapsed = true;
        signal();
      }, stallMs);
      try {
        await waitUntil(() => elapsed);
      } finally {
        clearTimeout(holdTimer);
      }
      if (failure) throw failure;
      if (client.records.length > client.cursor || client.closed)
        throw new Error("Client returned before containment deadline");
      terminal = true;
      transcript.push({ role: "supervisor", event: "deadline_terminal" });
      client.process.kill("SIGKILL");
      await waitUntil(() => client.closed);
      if (client.records.length !== client.cursor)
        throw new Error("Client returned during deadline containment");
    } else if (scenario === "stop-inflight") {
      terminal = true;
      transcript.push({ role: "supervisor", event: "stop_terminal" });
    }
    send(provider, "release");
    await next(provider, "released");
    const returned = await next(provider, "provider_returned");
    if (returned.hresult !== 0) throw new Error("Provider return failed");
    if (scenario !== "provider-deadline") {
      const response = await next(client, "call_returned");
      if (response.hresult !== 0 || !response.marker)
        throw new Error("Client did not receive provider marker");
      if (terminal) rejectedLate++;
      else admitted++;
      await waitUntil(() => client.closed);
      if (client.code !== 0 || client.cursor !== client.records.length)
        throw new Error("Client exit/transcript mismatch");
    }
    send(provider, "stop");
    await next(provider, "stopping");
    await next(provider, "closed");
    await waitUntil(() => provider.closed);
    if (provider.code !== 0 || provider.cursor !== provider.records.length)
      throw new Error("Provider exit/transcript mismatch");
    result.outcome = "observation";
  } catch (error) {
    result.error = error.message;
  } finally {
    clearTimeout(timer);
    for (const item of children) if (!item.closed) item.process.kill("SIGKILL");
    let cleanupTimer;
    const done = await Promise.race([
      Promise.all(children.map((item) => item.done)).then(() => true),
      new Promise((resolve) => {
        cleanupTimer = setTimeout(() => resolve(false), 2000);
      }),
    ]);
    clearTimeout(cleanupTimer);
    result.cleanup = done ? "closed" : "termination-unconfirmed";
    if (!done) result.outcome = "failed";
    result.admitted_results = admitted;
    result.rejected_late_results = rejectedLate;
  }
  return result;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.platform !== "win32" || process.arch !== "x64" || process.argv.length !== 5) {
    throw new Error("Usage on Windows x64: supervisor.mjs fixture.exe probe.exe scenario");
  }
  const hash = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
  const paths = [process.argv[1], process.argv[2], process.argv[3]];
  const hashes = paths.map(hash);
  const result = await supervise(process.argv[2], process.argv[3], { scenario: process.argv[4] });
  result.provenance = {
    supervisor_sha256: hashes[0],
    fixture_sha256: hashes[1],
    probe_sha256: hashes[2],
    os_release: release(),
    architecture: process.arch,
    node_version: process.version,
  };
  if (paths.some((path, index) => hash(path) !== hashes[index])) {
    result.outcome = "failed";
    result.error = "Run inputs changed";
  }
  console.log(JSON.stringify(result));
  process.exitCode = result.outcome === "observation" ? 0 : 1;
}
