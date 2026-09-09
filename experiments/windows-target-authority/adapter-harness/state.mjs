// Experiment-local correlation only; never proof of native target identity.
export const terminalReasons = Object.freeze([
  "completed_observation",
  "cancelled",
  "timed_out",
  "authority_uncertain",
  "native_failed",
  "protocol_failed",
]);

export function createOperation({ operation, receipt, readSequence }) {
  const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
  if (
    typeof operation !== "string" ||
    typeof receipt !== "string" ||
    !uuid.test(operation) ||
    !uuid.test(receipt) ||
    typeof readSequence !== "string" ||
    !/^[1-9][0-9]{0,19}$/.test(readSequence) ||
    BigInt(readSequence) > 18446744073709551615n
  )
    throw new Error("Invalid read identity");
  let stage = "pending";
  let reason = null;
  let cleanup = "pending";
  let accepted = 0;
  let rejected = 0;
  const finish = (next) => {
    if (!terminalReasons.includes(next)) throw new Error("Invalid terminal reason");
    if (stage === "terminal") return false;
    if (next === "completed_observation" && (stage !== "reading" || accepted !== 1))
      throw new Error("Completion requires exactly one correlated result");
    stage = "terminal";
    reason = next;
    return true;
  };
  return Object.freeze({
    begin() {
      if (stage !== "pending") return false;
      stage = "reading";
      return true;
    },
    finish,
    accept(envelope) {
      const valid =
        envelope !== null &&
        typeof envelope === "object" &&
        !Array.isArray(envelope) &&
        Object.keys(envelope).sort().join(",") === "operation,payload,readSequence,receipt" &&
        envelope.operation === operation &&
        envelope.receipt === receipt &&
        envelope.readSequence === readSequence;
      if (stage !== "reading" || accepted !== 0 || !valid) {
        rejected = Math.min(rejected + 1, Number.MAX_SAFE_INTEGER);
        if (stage !== "terminal") finish("protocol_failed");
        return false;
      }
      accepted = 1;
      return true;
    },
    cleanup(next) {
      if (!["closed", "termination-unconfirmed"].includes(next) || stage !== "terminal")
        throw new Error("Cleanup requires terminal operation and explicit status");
      if (cleanup !== "pending") return false;
      cleanup = next;
      return true;
    },
    snapshot() {
      return Object.freeze({
        operation,
        receipt,
        readSequence,
        stage,
        reason,
        cleanup,
        accepted,
        rejected,
        productAdmission: false,
      });
    },
  });
}
