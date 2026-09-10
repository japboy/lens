// This validates an experiment record, not its authenticity or product safety.
export const EVIDENCE_VERSION = 1;
export const SCENARIOS = [
  "ordinary",
  "replace-before-root",
  "replace-before-capture",
  "replace-before-commit",
];
const MAX_BYTES = 64 * 1024;
function require(condition, message) {
  if (!condition) throw new Error(message);
}
function fields(value, keys) {
  require(value !== null && typeof value === "object" && !Array.isArray(value), "Expected object");
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  require(actual.length === expected.length &&
    actual.every((key, index) => key === expected[index]), "Unexpected or missing fields");
}
function text(value) {
  require(typeof value === "string" &&
    value.length > 0 &&
    value.length <= 256 &&
    [...value].every(
      (character) => character.codePointAt(0) >= 32 && character.codePointAt(0) !== 127,
    ), "Invalid bounded text");
}
function integer(value, min, max) {
  require(Number.isSafeInteger(value) && value >= min && value <= max, "Invalid integer");
}
function marker(value) {
  require(value === "A" || value === "B", "Invalid fixture marker");
}
function handle(value) {
  require(typeof value === "string" &&
    /^0x[0-9a-f]{1,16}$/u.test(value) &&
    BigInt(value) !== 0n, "Invalid handle");
}
function runtimeId(value) {
  require(Array.isArray(value) && value.length > 0 && value.length <= 64, "Invalid runtime ID");
  for (const part of value) integer(part, -2147483648, 2147483647);
}

export function inspectEvidence(input) {
  require(typeof input === "string" &&
    Buffer.byteLength(input, "utf8") <= MAX_BYTES, "Evidence exceeds input limit");
  const record = JSON.parse(input);
  fields(record, ["version", "source_sha", "environment", "scenario", "deadline_ms", "events"]);
  require(record.version === EVIDENCE_VERSION, "Unsupported evidence version");
  require(typeof record.source_sha === "string" &&
    /^[0-9a-f]{40}$/u.test(record.source_sha), "Invalid source SHA");
  fields(record.environment, [
    "os",
    "architecture",
    "os_build",
    "compiler",
    "sdk",
    "uia_provider",
    "fixture_build",
  ]);
  require(record.environment.os === "windows" &&
    record.environment.architecture === "x64", "Unsupported evidence host");
  for (const value of Object.values(record.environment)) text(value);
  require(SCENARIOS.includes(record.scenario), "Unsupported scenario");
  integer(record.deadline_ms, 1, 60000);
  require(Array.isArray(record.events) &&
    record.events.length >= 2 &&
    record.events.length <= 32, "Invalid event count");
  const seen = new Map();
  let previousTime = -1;
  for (const [index, event] of record.events.entries()) {
    const extra = {
      review: ["handle", "pid", "marker"],
      replace: ["handle", "pid", "marker"],
      root: ["hresult", "marker", "runtime_id"],
      capture: ["hresult", "marker"],
      probe: ["hresult", "equal", "runtime_id"],
      notification: ["delivery"],
      terminal: ["outcome"],
    }[event?.kind];
    require(Array.isArray(extra), "Unknown event kind");
    fields(event, ["step", "elapsed_ms", "kind", ...extra]);
    require(event.step === index, "Noncontiguous event order");
    integer(event.elapsed_ms, 0, 120000);
    require(event.elapsed_ms >= previousTime, "Nonmonotonic event time");
    previousTime = event.elapsed_ms;
    require(!seen.has(event.kind), "Duplicate event kind");
    seen.set(event.kind, event);
    if (event.kind === "review" || event.kind === "replace") {
      handle(event.handle);
      integer(event.pid, 1, 4294967295);
      marker(event.marker);
    }
    if (["root", "capture", "probe"].includes(event.kind)) {
      integer(event.hresult, -2147483648, 2147483647);
      if (event.hresult < 0) {
        for (const key of extra.filter((key) => key !== "hresult"))
          require(event[key] === null, "Failed API has successful facts");
      } else {
        if (event.kind !== "probe") marker(event.marker);
        if (event.kind !== "capture") runtimeId(event.runtime_id);
        if (event.kind === "probe")
          require(typeof event.equal === "boolean", "Invalid comparison result");
      }
    }
    if (event.kind === "notification")
      require(["delivered", "pending", "not-observed"].includes(
        event.delivery,
      ), "Invalid notification observation");
    if (event.kind === "terminal")
      require(["admitted", "rejected", "timeout", "stopped", "provider-error"].includes(
        event.outcome,
      ), "Unknown terminal outcome");
  }
  const review = seen.get("review");
  const terminal = seen.get("terminal");
  require(review?.step === 0 && review.marker === "A", "Missing initial review A");
  require(terminal?.step === record.events.length - 1, "Missing final terminal event");
  const replacement = seen.get("replace");
  const root = seen.get("root");
  const capture = seen.get("capture");
  const probe = seen.get("probe");
  if (record.scenario === "ordinary") require(!replacement, "Unexpected replacement");
  else if (replacement) {
    require(replacement.marker === "B", "Replacement must carry oracle B");
    const before = {
      "replace-before-root": root,
      "replace-before-capture": capture,
      "replace-before-commit": terminal,
    }[record.scenario];
    const after = {
      "replace-before-root": review,
      "replace-before-capture": root,
      "replace-before-commit": probe,
    }[record.scenario];
    require(after &&
      replacement.step > after.step &&
      (!before || replacement.step < before.step), "Replacement barrier mismatch");
  }
  if (capture) require(root && root.step < capture.step, "Capture before root");
  if (probe) require(capture && capture.step < probe.step, "Probe before capture");
  if (terminal.outcome === "timeout")
    require(terminal.elapsed_ms >= record.deadline_ms, "Premature timeout");
  if (terminal.outcome === "admitted") {
    require(root?.hresult >= 0 &&
      capture?.hresult >= 0 &&
      probe?.hresult >= 0 &&
      probe.equal, "Admission without successful acquisitions/comparison");
    require(terminal.elapsed_ms < record.deadline_ms, "Admission after deadline");
  }
  const counterexample =
    terminal.outcome === "admitted" &&
    (root.marker !== review.marker || capture.marker !== review.marker);
  const reuse = replacement ? BigInt(replacement.handle) === BigInt(review.handle) : null;
  // Never issue a safety pass. A valid, ordinary record is only an observation.
  const classification = counterexample
    ? "counterexample"
    : terminal.outcome === "timeout" ||
        terminal.outcome === "provider-error" ||
        (record.scenario !== "ordinary" && !reuse)
      ? "inconclusive"
      : "observation";
  return {
    version: EVIDENCE_VERSION,
    scope: "record-validation-only",
    classification,
    scenario: record.scenario,
    outcome: terminal.outcome,
    handle_reused: reuse,
    same_pid: replacement ? replacement.pid === review.pid : null,
    mixed_sources: terminal.outcome === "admitted" && root.marker !== capture.marker,
    native_authenticity_verified: false,
    product_admission_granted: false,
  };
}
