export function shouldApplySnapshot(currentRevision: number, nextRevision: number): boolean {
  return Number.isSafeInteger(nextRevision) && nextRevision >= 0 && nextRevision > currentRevision;
}
