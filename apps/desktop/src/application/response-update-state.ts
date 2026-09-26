import type { ResponseHistoryPresentation } from "./response-history-controller";

export interface ResponseUpdateState {
  readonly scopeId: string;
  readonly baselineSequence: number;
  readonly latestSequence: number;
  readonly acknowledgedSequence: number;
  readonly pendingCount: number;
  readonly latestResponseId?: string;
  readonly arrivalRevision: number;
}
export interface ResponseUpdateTarget {
  readonly scopeId: string;
  readonly responseId: string;
  readonly sequence: number;
}

/** Complete manifest arrivals, never cache hydration, drive update availability. */
export function synchronizeResponseUpdates(
  previous: ResponseUpdateState | undefined,
  history: ResponseHistoryPresentation | undefined,
): ResponseUpdateState | undefined {
  // A temporary connection gap does not erase the same scope's acknowledgement.
  if (!history) return previous;
  const latest = history.responses.at(-1);
  const sequence = latest?.sequence ?? 0;
  if (!previous || previous.scopeId !== history.scopeId) {
    return {
      scopeId: history.scopeId,
      baselineSequence: sequence,
      latestSequence: sequence,
      acknowledgedSequence: sequence,
      pendingCount: 0,
      latestResponseId: latest?.id,
      arrivalRevision: 0,
    };
  }
  if (sequence <= previous.latestSequence) return previous;
  // The first answer is ordinary completion, not an update to a prior answer.
  const first =
    previous.latestSequence === 0
      ? (history.responses[0]?.sequence ?? 0)
      : previous.baselineSequence;
  const acknowledged = Math.max(previous.acknowledgedSequence, first);
  const pendingCount = history.responses.filter(
    (response) => response.sequence > acknowledged,
  ).length;
  return {
    ...previous,
    baselineSequence: first,
    latestSequence: sequence,
    latestResponseId: latest?.id,
    acknowledgedSequence: acknowledged,
    pendingCount,
    arrivalRevision: previous.arrivalRevision + (pendingCount > previous.pendingCount ? 1 : 0),
  };
}

export function captureResponseUpdate(
  state: ResponseUpdateState | undefined,
): ResponseUpdateTarget | undefined {
  return state?.pendingCount && state.latestResponseId
    ? { scopeId: state.scopeId, responseId: state.latestResponseId, sequence: state.latestSequence }
    : undefined;
}

export function acknowledgeResponseUpdate(
  state: ResponseUpdateState | undefined,
  target: ResponseUpdateTarget,
  history: ResponseHistoryPresentation | undefined,
): ResponseUpdateState | undefined {
  if (
    !state ||
    target.sequence > state.latestSequence ||
    state.scopeId !== target.scopeId ||
    history?.scopeId !== target.scopeId ||
    !history.responses.some(
      (response) => response.id === target.responseId && response.sequence === target.sequence,
    )
  )
    return state;
  const acknowledgedSequence = Math.max(
    state.acknowledgedSequence,
    Math.min(target.sequence, state.latestSequence),
  );
  return {
    ...state,
    acknowledgedSequence,
    pendingCount: history.responses.filter((response) => response.sequence > acknowledgedSequence)
      .length,
  };
}

export function newResponseLabel(count: number): string {
  return `${count} new ${count === 1 ? "response" : "responses"}`;
}
