import assert from "node:assert/strict";
import { test } from "node:test";
import { inspectEvidence } from "./evidence.mjs";

// Entirely synthetic protocol fixtures. These are NOT native run evidence.
function fixture() {
  return {
    version: 1,
    source_sha: "a".repeat(40),
    environment: {
      os: "windows",
      architecture: "x64",
      os_build: "synthetic",
      compiler: "synthetic",
      sdk: "synthetic",
      uia_provider: "synthetic",
      fixture_build: "synthetic",
    },
    scenario: "ordinary",
    deadline_ms: 1000,
    events: [
      { kind: "review", handle: "0x1", pid: 1, marker: "A" },
      { kind: "root", hresult: 0, marker: "A", runtime_id: [1, 2] },
      { kind: "capture", hresult: 0, marker: "A" },
      { kind: "probe", hresult: 0, equal: true, runtime_id: [1, 2] },
      { kind: "terminal", outcome: "admitted" },
    ],
  };
}
function encode(record) {
  return JSON.stringify({
    ...record,
    events: record.events.map((event, step) => ({ step, elapsed_ms: step, ...event })),
  });
}
test("valid record cannot grant native authenticity or product acceptance", () => {
  const result = inspectEvidence(encode(fixture()));
  assert.equal(result.classification, "observation");
  assert.equal(result.native_authenticity_verified, false);
  assert.equal(result.product_admission_granted, false);
});
test("reports a mixed-source oracle counterexample without requiring a reuse observation", () => {
  const record = fixture();
  record.scenario = "replace-before-capture";
  record.events.splice(2, 0, { kind: "replace", handle: "0x2", pid: 1, marker: "B" });
  record.events[3].marker = "B";
  const result = inspectEvidence(encode(record));
  assert.equal(result.classification, "counterexample");
  assert.equal(result.mixed_sources, true);
  assert.equal(result.handle_reused, false);
});
test("reuse not observed and incomplete attempt are inconclusive", () => {
  for (const replacement of [[], [{ kind: "replace", handle: "0x2", pid: 1, marker: "B" }]]) {
    const record = fixture();
    record.scenario = "replace-before-root";
    record.events = [record.events[0], ...replacement, { kind: "terminal", outcome: "rejected" }];
    assert.equal(inspectEvidence(encode(record)).classification, "inconclusive");
  }
});
test("reused-handle different-PID rejection is observation, never admission", () => {
  const record = fixture();
  record.scenario = "replace-before-root";
  record.events.splice(1, 0, { kind: "replace", handle: "0x01", pid: 2, marker: "B" });
  record.events.at(-1).outcome = "rejected";
  const result = inspectEvidence(encode(record));
  assert.equal(result.classification, "observation");
  assert.equal(result.handle_reused, true);
  assert.equal(result.same_pid, false);
  assert.equal(result.outcome, "rejected");
  assert.equal(result.product_admission_granted, false);
});

test("comma-containing keys cannot impersonate multiple metadata fields", () => {
  const record = fixture();
  delete record.environment.compiler;
  delete record.environment.fixture_build;
  record.environment["compiler,fixture_build"] = "synthetic";
  assert.throws(() => inspectEvidence(encode(record)), /Unexpected or missing fields/u);
});
test("timeout is inconclusive, not successful cancellation", () => {
  const record = fixture();
  record.events = [record.events[0], { kind: "terminal", outcome: "timeout", elapsed_ms: 1000 }];
  assert.equal(inspectEvidence(encode(record)).classification, "inconclusive");
  record.events[1].elapsed_ms = 999;
  assert.throws(() => inspectEvidence(encode(record)), /Premature timeout/u);
});
test("rejects unknown schemas, keys, host and unbounded or malformed values", () => {
  for (const mutate of [
    (r) => {
      r.version = 2;
    },
    (r) => {
      r.extra = true;
    },
    (r) => {
      r.environment.architecture = "arm64";
    },
    (r) => {
      r.source_sha = "not-a-sha";
    },
    (r) => {
      r.deadline_ms = 60001;
    },
    (r) => {
      r.scenario = "unknown";
    },
    (r) => {
      r.events[1].runtime_id = [];
    },
    (r) => {
      r.events[1].hresult = 2147483648;
    },
    (r) => {
      r.events[0].handle = "0x0";
    },
    (r) => {
      r.events[0].pid = 0;
    },
    (r) => {
      r.environment.sdk = "x".repeat(257);
    },
    (r) => {
      r.events[1].kind = "__proto__";
    },
  ]) {
    const record = fixture();
    mutate(record);
    assert.throws(() => inspectEvidence(encode(record)));
  }
  assert.throws(() => inspectEvidence(" ".repeat(65537)));
});
test("rejects reordered or repeated events and inconsistent success", () => {
  for (const mutate of [
    (r) => {
      r.events[1].step = 9;
    },
    (r) => {
      r.events[2].elapsed_ms = 0;
    },
    (r) => {
      r.events.splice(2, 0, { ...r.events[1] });
    },
    (r) => {
      r.events[1].hresult = -1;
    },
    (r) => {
      r.events[3].equal = false;
    },
    (r) => {
      r.events[4].elapsed_ms = 1000;
    },
    (r) => {
      r.scenario = "replace-before-capture";
      r.events.splice(1, 0, { kind: "replace", handle: "0x1", pid: 1, marker: "B" });
    },
  ]) {
    const record = fixture();
    mutate(record);
    assert.throws(() => inspectEvidence(encode(record)));
  }
});
test("failed APIs carry null facts and cannot authorize admission", () => {
  const record = fixture();
  record.events = [
    record.events[0],
    { kind: "root", hresult: -2147467259, marker: null, runtime_id: null },
    { kind: "terminal", outcome: "provider-error" },
  ];
  assert.equal(inspectEvidence(encode(record)).classification, "inconclusive");
  record.events.at(-1).outcome = "admitted";
  assert.throws(() => inspectEvidence(encode(record)));
});
