import { test } from "node:test";
import assert from "node:assert/strict";
import { parseRecord, supervise } from "./supervisor.mjs";

// Synthetic protocol peers only: these tests never claim UIA execution.
const fixture = `let seq=0;const emit=event=>console.log(JSON.stringify({version:1,event,seq:++seq,pid:process.pid,hresult:0}));emit('ready');require('node:readline').createInterface({input:process.stdin}).on('line',line=>{if(line==='arm'){emit('armed');setImmediate(()=>emit('provider_entered'));}else if(line==='release'){emit('released');emit('provider_returned');}else if(line==='stop'){emit('stopping');emit('closed');process.exit(0);}});`;
const probe = `let seq=0;const emit=(event,marker=false)=>console.log(JSON.stringify({version:1,event,seq:++seq,pid:process.pid,hresult:0,marker}));emit('root_ready');require('node:readline').createInterface({input:process.stdin}).on('line',()=>{emit('call_started');setTimeout(()=>{emit('call_returned',true);process.exit(0)},150);});`;
for (const scenario of ["release", "provider-deadline", "stop-inflight"]) {
  test(`synthetic finite protocol: ${scenario}`, async () => {
    const result = await supervise(process.execPath, process.execPath, {
      fixtureArgs: ["-e", fixture],
      probePrefixArgs: ["-e", probe],
      scenario,
      deadlineMs: 3000,
      stallMs: 25,
    });
    assert.equal(result.outcome, "observation", JSON.stringify(result));
    assert.equal(result.provider_entered, true);
    assert.equal(result.cleanup, "closed");
    assert.equal(result.admitted_results, scenario === "release" ? 1 : 0);
    assert.equal(result.rejected_late_results, scenario === "stop-inflight" ? 1 : 0);
    assert.equal(result.product_admission, false);
  });
}
const valid = { version: 1, event: "ready", seq: 1, pid: 123, hresult: 0 };
for (const mutation of [
  { version: 2 },
  { pid: 124 },
  { seq: 2 },
  { event: "invented" },
  { hresult: 1 },
  { extra: true },
  { hresult: -2147483649 },
]) {
  test(`strict native record ${JSON.stringify(mutation)}`, () => {
    assert.throws(() => parseRecord(JSON.stringify({ ...valid, ...mutation }), "fixture", 123, 1));
  });
}
test("missing actual provider entry cannot count as stall evidence", async () => {
  const result = await supervise(process.execPath, process.execPath, {
    fixtureArgs: ["-e", fixture.replace("setImmediate(()=>emit('provider_entered'));", "")],
    probePrefixArgs: ["-e", probe],
    scenario: "provider-deadline",
    deadlineMs: 300,
    stallMs: 25,
  });
  assert.equal(result.outcome, "failed");
  assert.equal(result.provider_entered, false);
  assert.equal(result.cleanup, "closed");
});
test("oversized output fails and closes owned process", async () => {
  const result = await supervise(process.execPath, process.execPath, {
    fixtureArgs: ["-e", "process.stdout.write('x'.repeat(70000));setInterval(()=>{},1000)"],
    deadlineMs: 1000,
    stallMs: 25,
  });
  assert.equal(result.outcome, "failed");
  assert.equal(result.cleanup, "closed");
});
test("external deadline interrupts an overlapping provider hold", async () => {
  // Reserve 3 s for both child startups instead of 250 ms. The hold cannot
  // naturally finish before 8.5 s (3 s entry delay + 5.5 s hold), while the
  // external deadline fires at 6 s and has a 2 s cleanup observation budget.
  // Keeping the completion bound below 8.5 s still detects an uninterruptible hold.
  const start = performance.now();
  const delayed = fixture.replace(
    "setImmediate(()=>emit('provider_entered'))",
    "setTimeout(()=>emit('provider_entered'),3000)",
  );
  const longProbe = probe.replace("},150)", "},30000)");
  const result = await supervise(process.execPath, process.execPath, {
    fixtureArgs: ["-e", delayed],
    probePrefixArgs: ["-e", longProbe],
    scenario: "provider-deadline",
    deadlineMs: 6000,
    stallMs: 5500,
  });
  assert.equal(result.provider_entered, true);
  assert.equal(result.outcome, "failed");
  assert.equal(result.cleanup, "closed");
  assert.match(result.error, /External experiment deadline/);
  assert.equal(
    result.transcript.some((record) => record.event === "deadline_terminal"),
    false,
  );
  assert.equal(
    result.transcript.some((record) => record.event === "released"),
    false,
  );
  assert.ok(
    performance.now() - start < 8000,
    "hold must not defer child cleanup until its own end",
  );
});
