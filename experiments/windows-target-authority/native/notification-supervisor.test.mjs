import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { inspectNotification, superviseNotifications } from "./notification-supervisor.mjs";

test("native source confines window mutations to owning UI thread, not cached HWND cleanup", async () => {
  // Structural regression guard only, not Windows runtime/lifetime evidence.
  const source = await readFile(new URL("./notification-probe.cpp", import.meta.url), "utf8");
  const ui = source.slice(
    source.indexOf("DWORD WINAPI FixtureThread"),
    source.indexOf("struct Event"),
  );
  const main = source.slice(source.indexOf("int main"));
  assert.doesNotMatch(source, /Post(?:Thread)?MessageW/);
  assert.match(ui, /DestroyWindow\(ownedWindow\)/);
  assert.match(source, /WM_NCDESTROY && target == ownedWindow/);
  assert.match(source, /ownedWindow = nullptr;\s*window = nullptr;/);
  assert.doesNotMatch(main, /DestroyWindow|SetWindowTextW|NotifyWinEvent/);
  assert.match(main, /stopRequested = true;\s*if \(requested && !SetEvent\(requested\)\)/);
  assert.match(main, /if \(joined && requested\) CloseHandle\(requested\)/);
});

const record = {
  version: 1,
  scope: "native-notification-observation",
  scenario: "queued",
  stage: "observed",
  hresult: 0,
  pid: 123,
  provider_description: "synthetic provider",
  framework_id: "synthetic framework",
  queued_before_fence: 1,
  events: [
    { sequence: 0, elapsed_ms: 1, phase: 0 },
    { sequence: 1, elapsed_ms: 2, phase: 2 },
  ],
  product_admission_granted: false,
};
test("synthetic notification parser preserves observed queued and late phases", () => {
  assert.equal(inspectNotification(record, "queued", 123, 4), record);
});
for (const [name, patch] of [
  ["wrong PID", { pid: 124 }],
  ["missing metadata", { provider_description: null }],
  ["wrong pending count", { queued_before_fence: 0 }],
  ["oversized metadata", { framework_id: "x".repeat(1025) }],
  ["wrong event sequence", { events: [{ sequence: 1, elapsed_ms: 1, phase: 0 }] }],
  [
    "backward phase",
    {
      events: [
        { sequence: 0, elapsed_ms: 1, phase: 2 },
        { sequence: 1, elapsed_ms: 2, phase: 0 },
      ],
    },
  ],
  [
    "backward time",
    {
      events: [
        { sequence: 0, elapsed_ms: 2, phase: 0 },
        { sequence: 1, elapsed_ms: 1, phase: 2 },
      ],
    },
  ],
  ["unknown fields", { admission: true }],
])
  test(`rejects ${name}`, () =>
    assert.throws(() => inspectNotification({ ...record, ...patch }, "queued", 123, 4)));

const synthetic = `const barriers=['registered','change-requested','unsubscribed','settled'];let i=0;
function next(){if(i<4)console.log(JSON.stringify({version:1,scope:'notification-barrier',barrier:barriers[i++]}));else{
console.log(JSON.stringify({...${JSON.stringify(record)},pid:process.pid,scenario:process.argv[1]}));process.exit(0);}}
require('node:readline').createInterface({input:process.stdin}).on('line',line=>{if(line!=='continue')process.exit(2);next();});next();`;
for (const scenario of ["queued", "unsubscribe-race"])
  test(`synthetic direct child ${scenario}`, async () => {
    const result = await superviseNotifications(process.execPath, {
      scenario,
      prefixArgs: ["-e", synthetic],
    });
    assert.equal(result.outcome, "observation");
    assert.equal(result.late_delivery, "observed");
    assert.equal(result.product_admission_granted, false);
  });
test("no callback within bounded run is inconclusive, never loss", async () => {
  const result = await superviseNotifications(process.execPath, {
    prefixArgs: [
      "-e",
      synthetic.replace("pid:process.pid,", "events:[],queued_before_fence:0,pid:process.pid,"),
    ],
  });
  assert.equal(result.outcome, "inconclusive");
  assert.equal(result.notification_delivery, "not-observed-within-window");
});
for (const [name, source, options] of [
  ["deadline", "setInterval(()=>{},1000)", { deadlineMs: 100 }],
  ["stdout cap", "console.log('x'.repeat(65537))", {}],
  ["stderr", "console.error('failure');process.exit(1)", {}],
  ["malformed", "console.log('not-json')", {}],
  ["partial", "process.stdout.write('{')", {}],
  ["wrong barrier", synthetic.replace("'registered'", "'unsubscribed'"), {}],
])
  test(`supervisor rejects ${name}`, async () => {
    await assert.rejects(
      superviseNotifications(process.execPath, { ...options, prefixArgs: ["-e", source] }),
    );
  });
test("supervisor rejects spawn failure", async () => {
  await assert.rejects(superviseNotifications("/nonexistent/lens-notification-probe"));
});
