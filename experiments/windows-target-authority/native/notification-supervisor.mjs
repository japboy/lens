import { spawn } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const barriers = ["registered", "change-requested", "unsubscribed", "settled"];
export function inspectNotification(record, scenario, pid, completedBarriers) {
  const keys = [
    "version",
    "scope",
    "scenario",
    "stage",
    "hresult",
    "pid",
    "provider_description",
    "framework_id",
    "queued_before_fence",
    "events",
    "product_admission_granted",
  ]
    .sort()
    .join();
  const stages = [
    "fixture",
    "mta",
    "root",
    "provider",
    "register",
    "change",
    "unsubscribe",
    "post-unsubscribe-change",
    "observed",
    "cleanup",
  ];
  if (
    !record ||
    Object.keys(record).sort().join() !== keys ||
    record.version !== 1 ||
    record.scope !== "native-notification-observation" ||
    record.scenario !== scenario ||
    record.pid !== pid ||
    record.product_admission_granted !== false ||
    !stages.includes(record.stage) ||
    !Number.isInteger(record.hresult) ||
    record.hresult < -2147483648 ||
    record.hresult > 0 ||
    !Number.isInteger(record.queued_before_fence) ||
    record.queued_before_fence < 0 ||
    record.queued_before_fence > 32 ||
    !Array.isArray(record.events) ||
    record.events.length > 32
  )
    throw new Error("Invalid notification result");
  for (const key of ["provider_description", "framework_id"]) {
    if (!(record[key] === null || (typeof record[key] === "string" && record[key].length <= 1024)))
      throw new Error("Invalid provider metadata");
  }
  let time = -1,
    phase = 0;
  for (const [index, event] of record.events.entries()) {
    if (
      !event ||
      Object.keys(event).sort().join() !== "elapsed_ms,phase,sequence" ||
      event.sequence !== index ||
      !Number.isInteger(event.elapsed_ms) ||
      event.elapsed_ms < time ||
      event.elapsed_ms < 0 ||
      event.elapsed_ms > 35000 ||
      !Number.isInteger(event.phase) ||
      event.phase < phase ||
      event.phase > 2
    )
      throw new Error("Invalid notification order");
    time = event.elapsed_ms;
    phase = event.phase;
  }
  if (
    record.hresult === 0 &&
    (record.stage !== "observed" ||
      completedBarriers !== 4 ||
      record.provider_description === null ||
      record.framework_id === null ||
      record.queued_before_fence !== record.events.filter((event) => event.phase === 0).length)
  )
    throw new Error("Incomplete notification success");
  if (record.hresult < 0 && record.stage === "observed")
    throw new Error("Inconsistent notification failure");
  return record;
}

/** Only owns its direct probe child; that child owns its synthetic UI thread/window. */
export function superviseNotifications(
  executable,
  { scenario = "queued", prefixArgs = [], deadlineMs = 35000, byteCap = 65536 } = {},
) {
  if (
    !["queued", "unsubscribe-race"].includes(scenario) ||
    !Number.isInteger(deadlineMs) ||
    deadlineMs < 1 ||
    deadlineMs > 35000 ||
    !Number.isInteger(byteCap) ||
    byteCap < 1 ||
    byteCap > 65536
  )
    return Promise.reject(new Error("Invalid notification supervisor options"));
  return new Promise((done, reject) => {
    const child = spawn(executable, [...prefixArgs, scenario], {
      shell: false,
      stdio: ["pipe", "pipe", "pipe"],
    });
    let failure, record;
    let pending = "",
      bytes = 0,
      stderr = 0,
      step = 0;
    const fail = (message) => {
      failure ??= new Error(message);
      child.kill("SIGKILL");
    };
    const timer = setTimeout(() => fail("Notification deadline exceeded"), deadlineMs);
    child.on("error", (error) => {
      failure ??= error;
    });
    child.stdin.on("error", (error) => fail(error.message));
    child.stderr.on("data", (chunk) => {
      stderr += chunk.length;
      if (stderr > byteCap) fail("Notification stderr cap exceeded");
    });
    child.stdout.on("data", (chunk) => {
      bytes += chunk.length;
      if (bytes > byteCap) {
        fail("Notification stdout cap exceeded");
        return;
      }
      pending += chunk.toString("utf8");
      for (;;) {
        const end = pending.indexOf("\n");
        if (end < 0 || failure) break;
        const line = pending.slice(0, end);
        pending = pending.slice(end + 1);
        try {
          const message = JSON.parse(line);
          if (record) throw new Error("Output after notification result");
          if (message.scope === "notification-barrier") {
            if (
              Object.keys(message).sort().join() !== "barrier,scope,version" ||
              message.version !== 1 ||
              message.barrier !== barriers[step]
            )
              throw new Error("Invalid notification barrier");
            step++;
            child.stdin.write("continue\n");
          } else {
            record = inspectNotification(message, scenario, child.pid, step);
            child.stdin.end();
          }
        } catch (error) {
          fail(error.message);
        }
      }
    });
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      if (failure) {
        reject(failure);
        return;
      }
      if (!record || pending || stderr || signal || code !== (record.hresult < 0 ? 1 : 0)) {
        reject(new Error("Incomplete notification transcript"));
        return;
      }
      done({
        version: 1,
        scope: "supervised-notification-experiment",
        product_admission_granted: false,
        deadline_ms: deadlineMs,
        outcome:
          record.hresult < 0 ? "failed" : record.events.length ? "observation" : "inconclusive",
        notification_delivery: record.events.length ? "observed" : "not-observed-within-window",
        late_delivery: record.events.some((event) => event.phase === 2)
          ? "observed"
          : "not-observed-within-window",
        barriers: barriers.slice(0, step),
        observation: record,
      });
    });
  });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.platform !== "win32" || process.arch !== "x64" || process.argv.length !== 4)
      throw new Error(
        "Requires Windows x64: notification-supervisor.mjs <probe.exe> <queued|unsubscribe-race>",
      );
    console.log(
      JSON.stringify(
        await superviseNotifications(resolve(process.argv[2]), { scenario: process.argv[3] }),
      ),
    );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
