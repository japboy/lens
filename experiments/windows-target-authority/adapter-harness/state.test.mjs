import { test } from "node:test";
import assert from "node:assert/strict";
import { createOperation, terminalReasons } from "./state.mjs";

const identity = {
  operation: "11111111-1111-4111-8111-111111111111",
  receipt: "22222222-2222-4222-8222-222222222222",
  readSequence: "1",
};
const result = () => ({ ...identity, payload: { synthetic: true } });

test("one correlated observation completes without product admission", () => {
  const op = createOperation(identity);
  assert.equal(op.begin(), true);
  assert.equal(op.begin(), false);
  assert.equal(op.accept(result()), true);
  assert.equal(op.finish("completed_observation"), true);
  assert.equal(op.cleanup("closed"), true);
  assert.equal(op.snapshot().productAdmission, false);
  assert.equal(op.begin(), false);
  assert.equal(op.accept(result()), false);
  assert.equal(op.snapshot().rejected, 1);
  assert.equal(op.snapshot().reason, "completed_observation");
});

for (const reason of terminalReasons.filter((r) => r !== "completed_observation")) {
  for (const reading of [false, true])
    test(`${reason} from ${reading ? "reading" : "pending"} is irreversible`, () => {
      const op = createOperation(identity);
      if (reading) op.begin();
      assert.equal(op.finish(reason), true);
      assert.equal(op.begin(), false);
      assert.equal(op.accept(result()), false);
      assert.equal(op.finish("cancelled"), false);
      assert.equal(op.snapshot().reason, reason);
      op.cleanup("termination-unconfirmed");
      assert.equal(op.cleanup("closed"), false);
      assert.equal(op.snapshot().cleanup, "termination-unconfirmed");
    });
}

for (const field of ["operation", "receipt", "readSequence"])
  test(`mismatched ${field} closes admission`, () => {
    const op = createOperation(identity);
    op.begin();
    assert.equal(op.accept({ ...result(), [field]: "stale" }), false);
    assert.equal(op.snapshot().reason, "protocol_failed");
    assert.equal(op.accept(result()), false);
  });

test("duplicate before completion fails rather than publishing twice", () => {
  const op = createOperation(identity);
  op.begin();
  assert.equal(op.accept(result()), true);
  assert.equal(op.accept(result()), false);
  assert.equal(op.snapshot().reason, "protocol_failed");
});

test("malformed and pending results fail closed", () => {
  for (const envelope of [null, [], {}, { ...result(), extra: true }]) {
    const op = createOperation(identity);
    op.begin();
    assert.equal(op.accept(envelope), false);
  }
  const op = createOperation(identity);
  assert.equal(op.accept(result()), false);
  assert.throws(() => createOperation({ ...identity, readSequence: "18446744073709551616" }));
  assert.throws(() => createOperation({ ...identity, readSequence: "01" }));
});

test("completion and cleanup cannot bypass acquisition", () => {
  const op = createOperation(identity);
  assert.throws(() => op.finish("completed_observation"));
  assert.throws(() => op.cleanup("closed"));
  assert.throws(() => op.finish("unknown"));
  op.begin();
  assert.throws(() => op.finish("completed_observation"));
  assert.equal(Object.isFrozen(op.snapshot()), true);
});

test("identity must be primitive strings, not coerced mutable objects", () => {
  for (const field of ["operation", "receipt"]) {
    assert.throws(() =>
      createOperation({ ...identity, [field]: { toString: () => identity[field] } }),
    );
    assert.throws(() => createOperation({ ...identity, [field]: null }));
  }
});
