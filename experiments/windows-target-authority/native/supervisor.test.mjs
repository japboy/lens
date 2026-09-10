import test from "node:test";
import assert from "node:assert/strict";
import { superviseFixture, superviseAcquisition } from "./supervisor.mjs";

// Synthetic Node children test process/protocol mechanics only, never Windows APIs.
const synthetic = `
let step=0,marker='none',handle='0x0';
function emit(event) { console.log(JSON.stringify({version:1,scope:'controlled-fixture-observation',step:step++,elapsed_ms:step,event,marker,handle,pid:process.pid,win32_error:0,product_admission_granted:false})); }
emit('ready');
require('node:readline').createInterface({input:process.stdin}).on('line',command=>{
 if(command==='create-a'){marker='A';handle='0x1';emit('created');}
 else if(command==='replace-b'){marker='B';handle='0x2';emit('created');emit('handle_not_reused');}
 else if(command==='stop'){marker='none';handle='0x0';emit('stopped');process.exit(0);}
 else emit(command);
});`;

test("portable synthetic child completes finite fixture protocol", async () => {
  const result = await superviseFixture(process.execPath, ["-e", synthetic]);
  assert.equal(result.observations.length, 9);
  assert.equal(result.native_acquisition_executed, false);
});

for (const [name, source, options] of [
  ["timeout", "setInterval(()=>{},1000)", { deadlineMs: 100 }],
  ["nonzero", "process.exit(2)", {}],
  ["incomplete", "process.exit(0)", {}],
  ["malformed", "console.log('{}')", {}],
  ["stdout cap", "process.stdout.write('x'.repeat(4096))", { byteCap: 128 }],
  ["stderr cap", "process.stderr.write('x'.repeat(4096))", { byteCap: 128 }],
  ["error observation", synthetic.replace("win32_error:0", "win32_error:5"), {}],
  [
    "negative initial time",
    synthetic.replace("elapsed_ms:step", "elapsed_ms:step===1?-1:step"),
    {},
  ],
  [
    "array ready handle",
    synthetic.replace(
      "event,marker,handle,pid",
      "event,marker,handle:event==='ready'?[handle]:handle,pid",
    ),
    {},
  ],
  [
    "array stopped handle",
    synthetic.replace(
      "event,marker,handle,pid",
      "event,marker,handle:event==='stopped'?[handle]:handle,pid",
    ),
    {},
  ],
  [
    "trailing partial",
    synthetic.replace("emit('stopped');", "emit('stopped');process.stdout.write('partial');"),
    {},
  ],
]) {
  test(`rejects ${name}`, async () => {
    await assert.rejects(superviseFixture(process.execPath, ["-e", source], options));
  });
}

test("rejects spawn failure", async () => {
  await assert.rejects(superviseFixture("/nonexistent/lens-fixture-test"));
});

const syntheticProbe = `
const barriers=['before-root','before-capture','before-probe','before-commit'];let i=0;
function next(){if(i<4)console.log(JSON.stringify({version:1,scope:'acquisition-barrier',barrier:barriers[i++]}));
else{console.log(JSON.stringify({version:1,scope:'isolated-acquisition-observation',stage:'observed',elapsed_ms:1,root_runtime_id:[1],probe_runtime_id:[1],equal:true,width:480,height:320,bgra:[220,80,30,255],hresult:0,sampled:true,marker:'A',product_admission_granted:false}));process.exit(0);}}
require('node:readline').createInterface({input:process.stdin}).on('line',line=>{if(line!=='continue')process.exit(2);next();});next();`;
for (const scenario of [
  "ordinary",
  "replace-before-root",
  "replace-before-capture",
  "replace-before-probe",
  "replace-before-commit",
]) {
  test(`synthetic acquisition keeps one probe across ${scenario}`, async () => {
    const result = await superviseAcquisition(process.execPath, process.execPath, {
      scenario,
      fixtureArgs: ["-e", synthetic],
      probePrefixArgs: ["-e", syntheticProbe],
    });
    assert.equal(result.product_admission_granted, false);
    assert.equal(result.outcome, scenario === "ordinary" ? "observation" : "inconclusive");
  });
}
test("acquisition deadline kills owned probe and fixture", async () => {
  await assert.rejects(
    superviseAcquisition(process.execPath, process.execPath, {
      deadlineMs: 200,
      fixtureArgs: ["-e", synthetic],
      probePrefixArgs: ["-e", "setInterval(()=>{},1000)"],
    }),
  );
});

test("PID negative control requires input-stage access denial", async () => {
  const denied = `console.log(JSON.stringify({version:1,scope:'isolated-acquisition-observation',stage:'input',elapsed_ms:1,root_runtime_id:null,probe_runtime_id:null,equal:null,width:0,height:0,bgra:[0,0,0,0],hresult:-2147024891,sampled:false,marker:'unknown',product_admission_granted:false}));process.exit(1);`;
  const result = await superviseAcquisition(process.execPath, process.execPath, {
    scenario: "mismatched-pid",
    fixtureArgs: ["-e", synthetic],
    probePrefixArgs: ["-e", denied],
  });
  assert.equal(result.expected_pid, process.pid);
  assert.notEqual(result.observations[0].pid, result.expected_pid);
  assert.equal(result.outcome, "failed");
  assert.equal(result.acquisition.stage, "input");
  await assert.rejects(
    superviseAcquisition(process.execPath, process.execPath, {
      scenario: "mismatched-pid",
      fixtureArgs: ["-e", synthetic],
      probePrefixArgs: ["-e", syntheticProbe],
    }),
    /PID negative control/,
  );
});

for (const barrier of ["root", "capture", "probe", "commit"]) {
  test(`Stop before ${barrier} aborts the retained probe without advancing`, async () => {
    const stoppable = syntheticProbe.replace(
      "if(line!=='continue')process.exit(2);",
      `if(line==='abort'){
      console.log(JSON.stringify({version:1,scope:'isolated-acquisition-observation',
      stage:['uia_create','uia_root_runtime_id','pixel_map','uia_compare'][i-1],elapsed_ms:1,
      root_runtime_id:i>1?[1]:null,probe_runtime_id:i>3?[1]:null,equal:i>3?true:null,
      width:i>2?480:0,height:i>2?320:0,bgra:i>2?[220,80,30,255]:[0,0,0,0],
      hresult:-2147467260,sampled:i>2,marker:i>2?'A':'unknown',product_admission_granted:false}));process.exit(1);
    }if(line!=='continue')process.exit(2);`,
    );
    const result = await superviseAcquisition(process.execPath, process.execPath, {
      scenario: `stop-before-${barrier}`,
      fixtureArgs: ["-e", synthetic],
      probePrefixArgs: ["-e", stoppable],
    });
    assert.equal(result.outcome, "stopped");
    assert.equal(result.observations.at(-1).event, "stopped");
    assert.equal(result.acquisition.hresult, -2147467260);
  });
}

for (const [name, from, to] of [
  ["missing stage", "stage:'observed',", ""],
  ["negative elapsed", "elapsed_ms:1", "elapsed_ms:-1"],
  ["invalid ID", "root_runtime_id:[1]", "root_runtime_id:[2147483648]"],
  ["missing comparison", "equal:true", "equal:null"],
  ["zero dimensions", "width:480", "width:0"],
  ["invalid BGRA", "220,80,30,255", "256,80,30,255"],
  ["wrong marker", "marker:'A'", "marker:'B'"],
  ["success at input", "stage:'observed'", "stage:'input'"],
  ["failure at observed", "hresult:0", "hresult:-1"],
])
  test(`acquisition rejects ${name}`, async () => {
    await assert.rejects(
      superviseAcquisition(process.execPath, process.execPath, {
        fixtureArgs: ["-e", synthetic],
        probePrefixArgs: ["-e", syntheticProbe.replace(from, to)],
      }),
    );
  });

test("acquisition rejects mismatched reuse event handle", async () => {
  await assert.rejects(
    superviseAcquisition(process.execPath, process.execPath, {
      scenario: "replace-before-root",
      fixtureArgs: [
        "-e",
        synthetic.replace("emit('handle_not_reused')", "handle='0x999';emit('handle_not_reused')"),
      ],
      probePrefixArgs: ["-e", syntheticProbe],
    }),
  );
});
