import { describe, expect, it } from "vitest";
import type { ResponseHistoryPresentation } from "./response-history-controller";
import {
  acknowledgeResponseUpdate,
  captureResponseUpdate,
  synchronizeResponseUpdates,
  type ResponseUpdateState,
  type ResponseUpdateTarget,
} from "./response-update-state";

function history(count: number, scopeId = "live-operation-a"): ResponseHistoryPresentation {
  return {
    scopeId,
    responses: Array.from({ length: count }, (_, index) => ({
      id: `response-${index + 1}`,
      sequence: index + 1,
      blocks: [{ type: "markdown", block_index: 0, byte_length: 10 }],
    })),
    media: [],
    htmlContents: new Map(),
    mediaErrors: new Map(),
    capacityReached: false,
  };
}

function withTwoUpdates(): ResponseUpdateState {
  const initial = synchronizeResponseUpdates(undefined, history(1));
  return synchronizeResponseUpdates(initial, history(3))!;
}

describe("response update acknowledgement", () => {
  it("does not create a signal before any manifest arrives", () => {
    expect(synchronizeResponseUpdates(undefined, undefined)).toBeUndefined();
    expect(captureResponseUpdate(undefined)).toBeUndefined();
  });

  it.each(["live-operation-a", "history:replay-generation-a"])(
    "treats the complete initial %s snapshot as a quiet baseline",
    (scopeId) => {
      const initial = synchronizeResponseUpdates(undefined, history(3, scopeId));
      expect(initial).toMatchObject({
        scopeId,
        baselineSequence: 3,
        latestSequence: 3,
        acknowledgedSequence: 3,
        latestResponseId: "response-3",
        pendingCount: 0,
        arrivalRevision: 0,
      });
      expect(captureResponseUpdate(initial)).toBeUndefined();
    },
  );

  it("keeps the first committed response quiet after an empty initial manifest", () => {
    const empty = synchronizeResponseUpdates(undefined, history(0));
    expect(empty?.pendingCount).toBe(0);
    expect(captureResponseUpdate(empty)).toBeUndefined();
    const first = synchronizeResponseUpdates(empty, history(1));
    expect(first).toMatchObject({
      baselineSequence: 1,
      latestSequence: 1,
      acknowledgedSequence: 1,
      pendingCount: 0,
      arrivalRevision: 0,
    });
    expect(captureResponseUpdate(first)).toBeUndefined();
    const appended = synchronizeResponseUpdates(first, history(3));
    expect(appended).toMatchObject({
      baselineSequence: 1,
      acknowledgedSequence: 1,
      latestSequence: 3,
      pendingCount: 2,
      arrivalRevision: 1,
    });
    expect(captureResponseUpdate(appended)).toEqual({
      scopeId: "live-operation-a",
      responseId: "response-3",
      sequence: 3,
    });
  });

  it("counts only later responses when a first-answer snapshot also includes two missed appends", () => {
    const empty = synchronizeResponseUpdates(undefined, history(0));
    const batched = synchronizeResponseUpdates(empty, history(3));
    expect(batched).toMatchObject({
      baselineSequence: 1,
      acknowledgedSequence: 1,
      latestSequence: 3,
      pendingCount: 2,
      arrivalRevision: 1,
    });
  });

  it("ignores duplicate, stale, capacity-only and lazily hydrated presentations", () => {
    const pending = withTwoUpdates();
    const hydrated: ResponseHistoryPresentation = {
      ...history(3),
      htmlContents: new Map([
        [
          "loaded-artifact",
          { resourceId: "loaded-artifact", status: "ready", content: "<p>Loaded</p>" },
        ],
      ]),
      mediaErrors: new Map([["other-artifact", "Could not load image"]]),
    };
    for (const presentation of [
      history(3),
      history(1),
      hydrated,
      { ...history(3), capacityReached: true },
    ]) {
      expect(synchronizeResponseUpdates(pending, presentation)).toBe(pending);
    }
    expect(pending.pendingCount).toBe(2);
    expect(pending.acknowledgedSequence).toBe(1);
    expect(pending.arrivalRevision).toBe(1);
  });

  it("retains acknowledgement through a temporary absent manifest and same-scope reconnect", () => {
    const pending = withTwoUpdates();
    const acknowledged = acknowledgeResponseUpdate(
      pending,
      captureResponseUpdate(pending)!,
      history(3),
    );
    expect(acknowledged?.pendingCount).toBe(0);
    const disconnected = synchronizeResponseUpdates(acknowledged, undefined);
    expect(disconnected).toBe(acknowledged);
    expect(synchronizeResponseUpdates(disconnected, history(3))).toBe(acknowledged);
    const next = synchronizeResponseUpdates(disconnected, history(4));
    expect(next).toMatchObject({ acknowledgedSequence: 3, pendingCount: 1, latestSequence: 4 });
  });

  it("resets the baseline on scope replacement and rejects an old captured target", () => {
    const pending = withTwoUpdates();
    const oldTarget = captureResponseUpdate(pending)!;
    const replacementHistory = history(2, "live-operation-b");
    const replacement = synchronizeResponseUpdates(pending, replacementHistory);
    expect(replacement).toMatchObject({
      scopeId: "live-operation-b",
      baselineSequence: 2,
      acknowledgedSequence: 2,
      latestSequence: 2,
      pendingCount: 0,
      arrivalRevision: 0,
    });
    expect(acknowledgeResponseUpdate(replacement, oldTarget, replacementHistory)).toBe(replacement);
  });

  it("acknowledges the captured response while leaving an arrival during navigation pending", () => {
    const pending = withTwoUpdates();
    const target = captureResponseUpdate(pending)!;
    const duringNavigation = synchronizeResponseUpdates(pending, history(4));
    expect(target).toEqual({ scopeId: "live-operation-a", responseId: "response-3", sequence: 3 });
    expect(duringNavigation?.pendingCount).toBe(3);
    const acknowledged = acknowledgeResponseUpdate(duringNavigation, target, history(4));
    expect(acknowledged).toMatchObject({
      latestSequence: 4,
      latestResponseId: "response-4",
      acknowledgedSequence: 3,
      pendingCount: 1,
      arrivalRevision: 2,
    });
    const laterTarget = captureResponseUpdate(acknowledged)!;
    expect(laterTarget.sequence).toBe(4);
    const finished = acknowledgeResponseUpdate(acknowledged, laterTarget, history(4));
    expect(finished?.pendingCount).toBe(0);
    expect(captureResponseUpdate(finished)).toBeUndefined();
    expect(acknowledgeResponseUpdate(finished, target, history(4))).toEqual(finished);
  });

  it.each<ResponseUpdateTarget>([
    { scopeId: "live-operation-a", responseId: "missing-response", sequence: 3 },
    { scopeId: "live-operation-a", responseId: "response-2", sequence: 3 },
    { scopeId: "live-operation-a", responseId: "response-3", sequence: 2 },
    { scopeId: "foreign-operation", responseId: "response-3", sequence: 3 },
    { scopeId: "live-operation-a", responseId: "response-3", sequence: -1 },
    { scopeId: "live-operation-a", responseId: "response-3", sequence: Number.NaN },
    { scopeId: "live-operation-a", responseId: "response-3", sequence: Number.POSITIVE_INFINITY },
  ])("ignores an invalid captured identity %#", (target) => {
    const pending = withTwoUpdates();
    expect(acknowledgeResponseUpdate(pending, target, history(3))).toBe(pending);
  });

  it("does not acknowledge a target newer than the state that could have captured it", () => {
    const pending = withTwoUpdates();
    const impossibleTarget = {
      scopeId: "live-operation-a",
      responseId: "response-4",
      sequence: 4,
    };
    expect(acknowledgeResponseUpdate(pending, impossibleTarget, history(4))).toBe(pending);
  });

  it("requires a current matching manifest to acknowledge navigation completion", () => {
    const pending = withTwoUpdates();
    const target = captureResponseUpdate(pending)!;
    for (const unavailable of [undefined, history(2), history(3, "history:other-generation")]) {
      expect(acknowledgeResponseUpdate(pending, target, unavailable)).toBe(pending);
    }
    expect(acknowledgeResponseUpdate(undefined, target, history(3))).toBeUndefined();
  });
});
