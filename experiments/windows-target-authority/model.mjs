import assert from "node:assert/strict";

// An adversarial model, NOT a Windows API emulator. True incarnation is
// test-oracle-only information: the proposed adapter cannot inspect it.
const steps = ["root", "capture", "probe", "commit"];
function interleave(left, right) {
  if (!left.length) return [right];
  if (!right.length) return [left];
  return [
    ...interleave(left.slice(1), right).map((rest) => [left[0], ...rest]),
    ...interleave(left, right.slice(1)).map((rest) => [right[0], ...rest]),
  ];
}

function run(trace, { samePid, sameRuntimeId, staleRootComparable }) {
  const selected = { incarnation: "A", pid: 1, runtimeId: 1 };
  const replacement = {
    incarnation: "B",
    pid: samePid ? 1 : 2,
    runtimeId: sameRuntimeId ? 1 : 2,
  };
  let current = selected;
  let root;
  let capture;
  let probePassed = false;
  let terminal = false;
  let result;
  for (const event of trace) {
    switch (event) {
      case "replace":
        current = replacement;
        break;
      case "notify":
        terminal = true;
        break;
      case "root":
        root = current;
        break;
      case "capture":
        capture = current;
        break;
      case "probe":
        probePassed =
          current.pid === selected.pid &&
          root.runtimeId === current.runtimeId &&
          (root === current || staleRootComparable);
        if (!probePassed) terminal = true;
        break;
      case "commit": {
        const admitted = !terminal && probePassed;
        result = {
          admitted,
          wrongTarget: admitted && (root !== selected || capture !== selected),
          mixedSources: admitted && root !== capture,
          root: root.incarnation,
          capture: capture.incarnation,
        };
        break;
      }
      default:
        throw new Error(`Unknown event: ${event}`);
    }
  }
  return result;
}

const baseline = { samePid: true, sameRuntimeId: true, staleRootComparable: true };
assert.deepEqual(run(steps, baseline), {
  admitted: true,
  wrongTarget: false,
  mixedSources: false,
  root: "A",
  capture: "A",
});
// Replacement before both acquisitions needs no stale-proxy assumption and
// does not even require runtime-ID reuse: both API calls resolve B.
const beforeRoot = ["replace", ...steps, "notify"];
assert.equal(
  run(beforeRoot, { ...baseline, sameRuntimeId: false, staleRootComparable: false }).wrongTarget,
  true,
);
// Mixed-source result depends on the explicitly adversarial stale-root model.
const betweenApis = ["root", "replace", "capture", "probe", "commit", "notify"];
assert.equal(run(betweenApis, baseline).mixedSources, true);
assert.equal(run(betweenApis, { ...baseline, staleRootComparable: false }).admitted, false);
assert.equal(run(beforeRoot, { ...baseline, samePid: false }).admitted, false);
assert.equal(
  run(["root", "replace", "notify", "capture", "probe", "commit"], baseline).admitted,
  false,
);

const traces = interleave(steps, ["replace", "notify"]);
assert.equal(traces.length, 15);
const rows = [];
for (const samePid of [false, true]) {
  for (const sameRuntimeId of [false, true]) {
    for (const staleRootComparable of [false, true]) {
      const assumptions = { samePid, sameRuntimeId, staleRootComparable };
      const results = traces.map((trace) => ({ trace, ...run(trace, assumptions) }));
      rows.push({
        ...assumptions,
        cases: results.length,
        admitted: results.filter((r) => r.admitted).length,
        wrongTarget: results.filter((r) => r.wrongTarget).length,
        mixedSources: results.filter((r) => r.mixedSources).length,
      });
    }
  }
}
assert.equal(
  rows.reduce((n, row) => n + row.cases, 0),
  120,
);
assert.ok(rows.some((row) => row.wrongTarget > 0));
console.log(
  JSON.stringify(
    {
      scope: "bounded admission model; not native Windows validation",
      schedulesPerAssumptionSet: traces.length,
      rows,
      counterexamples: [
        {
          trace: beforeRoot,
          assumptions: { ...baseline, sameRuntimeId: false, staleRootComparable: false },
        },
        { trace: betweenApis, assumptions: baseline },
      ].map(({ trace, assumptions }) => ({
        trace,
        assumptions,
        ...run(trace, assumptions),
      })),
    },
    null,
    2,
  ),
);
