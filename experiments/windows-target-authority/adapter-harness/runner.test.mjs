import { test } from "node:test";
import assert from "node:assert/strict";
import { runOperation } from "./runner.mjs";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";

// Protocol doubles only; no Windows API claims.
const fixture = `let seq=0;const emit=event=>console.log(JSON.stringify({version:1,event,seq:++seq,pid:process.pid,hresult:0}));emit('ready');require('node:readline').createInterface({input:process.stdin}).on('line',line=>{if(line==='arm'){emit('armed');setImmediate(()=>emit('provider_entered'));}else if(line==='release'){emit('released');emit('provider_returned');}else if(line==='stop'){emit('stopping');emit('closed');process.exit(0);}});`;
const probe = `let seq=0;const emit=(event,marker=false)=>console.log(JSON.stringify({version:1,event,seq:++seq,pid:process.pid,hresult:0,marker}));emit('root_ready');require('node:readline').createInterface({input:process.stdin}).on('line',()=>{emit('call_started');setTimeout(()=>{emit('call_returned',true);process.exit(0)},750);});`;
const options = {
  kind: "provider",
  fixture: process.execPath,
  probe: process.execPath,
  fixtureArgs: ["-e", fixture],
  probePrefixArgs: ["-e", probe],
  deadlineMs: 5000,
  stallMs: 100,
};

for (const [scenario, reason] of [
  ["release", "completed_observation"],
  ["stop-inflight", "cancelled"],
  ["provider-deadline", "timed_out"],
]) {
  test(`owned provider lifecycle ${scenario}`, async () => {
    const result = await runOperation({ ...options, scenario });
    assert.equal(result.reason, reason, JSON.stringify(result));
    assert.equal(result.cleanup, "closed");
    assert.equal(result.productAdmission, false);
    assert.equal(result.accepted, scenario === "release" ? 1 : 0);
    assert.equal(result.rejected, scenario === "stop-inflight" ? 1 : 0);
    assert.equal(result.published === null, scenario !== "release");
    assert.equal(result.native.children.length, 2);
    assert.ok(result.native.children.every((child) => child.closed && child.stdout.length));
    if (scenario === "stop-inflight") {
      assert.ok(
        result.transitions.findIndex((event) => event.event === "terminal") <
          result.transitions.findIndex((event) => event.event === "result"),
      );
    }
  });
}

for (const kind of ["provider", "acquisition"]) {
  test(`${kind} pre-start Stop creates no child and goes directly pending to terminal`, async () => {
    const result = await runOperation({
      ...options,
      kind,
      fixture: "/nonexistent/pre-start-fixture",
      probe: "/nonexistent/pre-start-probe",
      scenario: kind === "provider" ? "release" : "ordinary",
      cancelBeforeStart: true,
      spawnChild() {
        assert.fail("Pre-start cancellation must not spawn");
      },
    });
    assert.equal(result.reason, "cancelled");
    assert.equal(result.verification, "observed");
    assert.equal(result.error, undefined);
    assert.equal(result.cleanup, "closed");
    assert.deepEqual(result.native.children, []);
    assert.equal(
      result.transitions.some((entry) => entry.event === "begin"),
      false,
    );
  });
}

for (const [name, source, reason] of [
  ["missing handshake", "setInterval(()=>{},1000)", "timed_out"],
  ["malformed", "console.log('{}');setInterval(()=>{},1000)", "protocol_failed"],
  [
    "oversized",
    "process.stdout.write('x'.repeat(70000));setInterval(()=>{},1000)",
    "protocol_failed",
  ],
  ["partial", "process.stdout.write('{');", "protocol_failed"],
]) {
  test(`bounded failure: ${name}`, async () => {
    const result = await runOperation({
      ...options,
      scenario: "release",
      fixtureArgs: ["-e", source],
      deadlineMs: 1500,
    });
    assert.equal(result.reason, reason, JSON.stringify(result));
    assert.equal(result.cleanup, "closed");
    assert.equal(result.published, null);
    assert.ok(
      result.native.children.every((child) => child.closed && child.stdout.length <= 65536),
    );
  });
}

const windowFixture = `let step=0,marker='none',handle='0x0';function emit(event){console.log(JSON.stringify({version:1,scope:'controlled-fixture-observation',step:step++,elapsed_ms:step,event,marker,handle,pid:process.pid,win32_error:0,product_admission_granted:false}));}emit('ready');require('node:readline').createInterface({input:process.stdin}).on('line',command=>{if(command==='create-a'){marker='A';handle='0x1';emit('created');}else if(command==='replace-b'){marker='B';handle='0x2';emit('created');emit('handle_not_reused');}else if(command==='stop'){marker='none';handle='0x0';emit('stopped');process.exit(0);}else emit(command);});`;
const windowProbe = `const barriers=['before-root','before-capture','before-probe','before-commit'];let i=0;function emit(abort=false){console.log(JSON.stringify({version:1,scope:'isolated-acquisition-observation',stage:abort?['uia_create','uia_root_runtime_id','pixel_map','uia_compare'][i-1]:'observed',elapsed_ms:1,root_runtime_id:!abort||i>1?[1]:null,probe_runtime_id:!abort||i>3?[1]:null,equal:!abort||i>3?true:null,width:!abort||i>2?480:0,height:!abort||i>2?320:0,bgra:!abort||i>2?[220,80,30,255]:[0,0,0,0],hresult:abort?-2147467260:0,sampled:!abort||i>2,marker:!abort||i>2?'A':'unknown',product_admission_granted:false}));process.exit(abort?1:0);}function next(){if(i<4)console.log(JSON.stringify({version:1,scope:'acquisition-barrier',barrier:barriers[i++]}));else emit();}require('node:readline').createInterface({input:process.stdin}).on('line',line=>{if(line==='abort')emit(true);else if(line==='continue')next();else process.exit(2);});next();`;
for (const scenario of [
  "ordinary",
  "stop-before-root",
  "stop-before-capture",
  "stop-before-probe",
  "stop-before-commit",
  "replace-before-root",
  "replace-before-capture",
  "replace-before-probe",
  "replace-before-commit",
]) {
  test(`acquisition channel ${scenario}`, async () => {
    const result = await runOperation({
      kind: "acquisition",
      fixture: process.execPath,
      probe: process.execPath,
      scenario,
      fixtureArgs: ["-e", windowFixture],
      probePrefixArgs: ["-e", windowProbe],
    });
    assert.equal(
      result.reason,
      scenario === "ordinary"
        ? "completed_observation"
        : scenario.startsWith("stop")
          ? "cancelled"
          : "authority_uncertain",
      JSON.stringify(result),
    );
    assert.equal(result.cleanup, "closed");
    assert.equal(result.published === null, scenario !== "ordinary");
    assert.equal(result.native.children.length, 2);
    if (scenario !== "ordinary") {
      assert.equal(result.native.acquisition.hresult, -2147467260);
      assert.equal(result.rejected, 1);
      assert.equal(result.accepted, 0);
    }
  });
}

test("missing close observation stays unconfirmed despite termination request", async () => {
  let kills = 0;
  const child = Object.assign(new EventEmitter(), {
    pid: 123,
    stdin: new PassThrough(),
    stdout: new PassThrough(),
    stderr: new PassThrough(),
    kill() {
      kills++;
      return false;
    },
  });
  const result = await runOperation({
    ...options,
    scenario: "release",
    deadlineMs: 150,
    stallMs: 25,
    spawnChild: () => child,
  });
  assert.equal(result.reason, "timed_out");
  assert.equal(result.cleanup, "termination-unconfirmed");
  assert.equal(result.published, null);
  assert.equal(result.native.children[0].closed, false);
  assert.ok(kills >= 1);
  for (const stream of [child.stdin, child.stdout, child.stderr]) stream.destroy();
});

test("comparison mismatch rejects a syntactically valid acquisition before acceptance", async () => {
  const result = await runOperation({
    kind: "acquisition",
    fixture: process.execPath,
    probe: process.execPath,
    scenario: "ordinary",
    fixtureArgs: ["-e", windowFixture],
    probePrefixArgs: [
      "-e",
      windowProbe.replace("equal:!abort||i>3?true:null", "equal:!abort||i>3?false:null"),
    ],
  });
  assert.equal(result.reason, "authority_uncertain");
  assert.equal(result.accepted, 0);
  assert.equal(result.rejected, 1);
  assert.equal(result.published, null);
});

for (const [name, source] of [
  ["partial", "process.stdout.write('{')"],
  ["malformed", "console.log('{}')"],
  ["oversized", "process.stdout.write('x'.repeat(70000))"],
  ["nonzero result", windowProbe.replace("process.exit(abort?1:0)", "process.exit(2)")],
  [
    "duplicate result",
    windowProbe.replace("process.exit(abort?1:0)", "console.log('{}');process.exit(0)"),
  ],
]) {
  test(`acquisition ${name} retains child evidence but no publication`, async () => {
    const result = await runOperation({
      kind: "acquisition",
      fixture: process.execPath,
      probe: process.execPath,
      scenario: "ordinary",
      fixtureArgs: ["-e", windowFixture],
      probePrefixArgs: ["-e", source],
    });
    assert.ok(["protocol_failed", "native_failed"].includes(result.reason), JSON.stringify(result));
    assert.equal(result.cleanup, "closed");
    assert.equal(result.published, null);
    assert.equal(result.native.children.length, 2);
    assert.ok(result.native.children.every((entry) => entry.closed));
  });
}
