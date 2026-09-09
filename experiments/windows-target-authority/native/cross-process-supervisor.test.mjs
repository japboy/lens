import test from "node:test";
import assert from "node:assert/strict";
import { superviseCrossProcess } from "./cross-process-supervisor.mjs";

// Synthetic subprocess protocol tests; no Windows API or native evidence.
const fixture = `let step=0,marker='none',handle='0x0';
function emit(event){console.log(JSON.stringify({version:1,scope:'controlled-fixture-observation',step:step++,elapsed_ms:step,event,marker,handle,pid:process.pid,win32_error:0,product_admission_granted:false}));}
emit('ready');require('node:readline').createInterface({input:process.stdin}).on('line',line=>{
if(line==='create-a'){marker='A';handle='0x1';emit('created');}
else if(line==='replace-b'){marker='B';handle='0x1';emit('created');emit('handle_reused');}
else if(line==='stop'){marker='none';handle='0x0';emit('stopped');process.exit(0);}
else emit(line);});`;
const probe = `let i=0;const barriers=['before-root','before-capture','before-probe','before-commit'];
function result(abort){console.log(JSON.stringify({version:1,scope:'isolated-acquisition-observation',stage:abort?['uia_create','uia_root_runtime_id','pixel_map','uia_compare'][i-1]:'observed',hresult:abort?-2147467260:0,elapsed_ms:1,root_runtime_id:i>1?[1]:null,probe_runtime_id:i>3?[1]:null,equal:i>3?true:null,sampled:i>2,width:i>2?480:0,height:i>2?320:0,bgra:i>2?[220,80,30,255]:[0,0,0,0],marker:i>2?'A':'unknown',product_admission_granted:false}));process.exit(abort?1:0);}
function next(){if(i===4)result(false);else console.log(JSON.stringify({version:1,scope:'acquisition-barrier',barrier:barriers[i++]}));}
require('node:readline').createInterface({input:process.stdin}).on('line',line=>{if(line==='abort')result(true);else if(line==='continue')next();else process.exit(2);});next();`;
const options = { fixtureArgs: ["-e", fixture], probePrefixArgs: ["-e", probe] };
for (const scenario of ["replace-before-probe", "replace-before-commit"]) {
  test(`synthetic owned process replacement at ${scenario}`, async () => {
    const result = await superviseCrossProcess(process.execPath, process.execPath, {
      ...options,
      scenario,
    });
    assert.equal(result.outcome, "observation");
    assert.equal(result.product_admission_granted, false);
    assert.notEqual(result.original.pid, result.replacement.pid);
    assert.equal(result.original.handle, result.replacement.handle);
    assert.equal(result.fixture_observations.original.at(-1).event, "stopped");
    assert.equal(result.fixture_observations.replacement.at(-1).event, "stopped");
  });
  test(`synthetic non-reused HWND aborts at ${scenario}`, async () => {
    const replacement = fixture
      .replace("marker='B';handle='0x1'", "marker='B';handle='0x2'")
      .replace("emit('handle_reused')", "emit('handle_not_reused')");
    const result = await superviseCrossProcess(process.execPath, process.execPath, {
      ...options,
      scenario,
      replacementArgs: ["-e", replacement],
    });
    assert.equal(result.outcome, "inconclusive");
    assert.equal(result.owned_handle_reuse, false);
    assert.equal(result.acquisition.hresult, -2147467260);
    assert.equal(
      result.acquisition.stage,
      scenario === "replace-before-probe" ? "pixel_map" : "uia_compare",
    );
  });
}
for (const [name, changes] of [
  ["timeout", { probePrefixArgs: ["-e", "setInterval(()=>{},1000)"], deadlineMs: 150 }],
  ["stdout cap", { probePrefixArgs: ["-e", "process.stdout.write('x'.repeat(70000))"] }],
  ["stderr cap", { probePrefixArgs: ["-e", "process.stderr.write('x'.repeat(70000))"] }],
  ["wrong PID", { replacementArgs: ["-e", fixture.replace("pid:process.pid", "pid:1")] }],
  ["malformed probe", { probePrefixArgs: ["-e", "console.log('{}')"] }],
  ["incomplete probe", { probePrefixArgs: ["-e", "process.exit(0)"] }],
  ["false success stage", { probePrefixArgs: ["-e", probe.replace(":'observed'", ":'input'")] }],
  ["invalid scenario", { scenario: "replace-before-capture" }],
  ["invalid limits", { byteCap: 0 }],
])
  test(`cross-process rejects ${name}`, async () => {
    await assert.rejects(
      superviseCrossProcess(process.execPath, process.execPath, { ...options, ...changes }),
    );
  });
test("probe API failure remains a failed observation without a replacement", async () => {
  const denied = `console.log(JSON.stringify({version:1,scope:'isolated-acquisition-observation',stage:'input',hresult:-2147024891,elapsed_ms:1,root_runtime_id:null,probe_runtime_id:null,equal:null,sampled:false,width:0,height:0,bgra:[0,0,0,0],marker:'unknown',product_admission_granted:false}));process.exit(1);`;
  const result = await superviseCrossProcess(process.execPath, process.execPath, {
    ...options,
    probePrefixArgs: ["-e", denied],
  });
  assert.equal(result.outcome, "failed");
  assert.equal(result.replacement_performed, false);
});
test("cross-process spawn failure rejects and reaps owned children", async () => {
  await assert.rejects(
    superviseCrossProcess(process.execPath, "/nonexistent/lens-cross-probe", options),
  );
});
