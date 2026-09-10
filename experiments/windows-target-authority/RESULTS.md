# Admission Model Results

Executed 2026-09-07 on macOS (Darwin arm64), Node 24.19.0, against Lens
baseline `ee4c324`. Command: `node experiments/windows-target-authority/model.mjs`.
Exit status 0: all six scenario assertions and enumeration checks passed.
This is a successful counterexample experiment, NOT a safety acceptance pass.

## Evidence

The model enumerated 120 cases: 15 event interleavings for each of eight
assumption sets. Five cases admitted a different incarnation from the reviewed
target; one of those also mixed UIA(A) with capture(B). These counts are model
coverage, not occurrence probabilities or observations of Windows behavior.

1. Review A; replace it with B using the same handle and PID; acquire root(B),
   capture(B), compare with probe(B), commit, then deliver destruction notification.
   Both retained objects agree but neither belongs to reviewed A. This model
   counterexample needs neither runtime-ID reuse nor a usable stale UIA proxy.
2. Acquire root(A); replace A with B; acquire capture(B), compare, commit, then
   deliver notification. This mixed-source counterexample additionally assumes
   runtime-ID reuse and successful comparison of the stale root. Switching off
   stale-root comparison rejects this case. That switch is an unknown native
   behavior, not a discovered fact about Windows.

Controls verify ordinary admission, PID mismatch rejection, stale-root comparison
failure rejection, and notification-before-commit rejection. Acquisition failures,
multiple replacements, provider stalls and post-admission publication are outside
this experiment, so no claim is made about those paths.

## Interpretation And Next Gate

The proposed observable checks alone do not imply continuity from user review
through both native acquisitions in this abstraction. Retaining objects _after_
acquisition does not establish which reviewed incarnation was acquired. The
production safety requirement is unchanged; no replacement design is approved.

A Windows harness must instrument a controlled same-process A/B window fixture
with an oracle incarnation marker unavailable to production admission. Insert
barriers before root acquisition and between root/capture/probe/commit; destroy
and recreate windows, recording whether numeric handle reuse actually occurred.
Record UIA HRESULTs/runtime IDs, capture-source markers, and notification delivery
order. If reuse is not observed, classify that scenario as inconclusive, not pass.
Do not send any fixture content to a real Agent. Even many successful stress runs
do not prove a lifetime guarantee: an implementable lifecycle boundary backed by
API guarantees, or an explicitly approved scope restriction, remains required.

## Official Specification Boundary

- [IsWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-iswindow)
  explicitly warns about destruction after a check and recycled handles pointing
  to another window. Adding an existence check cannot by itself resolve this race.
- [ElementFromHandle](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation-elementfromhandle)
  accepts a handle and returns its UIA element; the documented contract does not
  provide a transaction spanning user review and capture creation.
- [CompareElements](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomation-compareelements)
  compares runtime identifiers; [GetRuntimeId](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationclient/nf-uiautomationclient-iuiautomationelement-getruntimeid)
  permits reuse over time. Neither establishes the stale-proxy behavior modeled
  above. This experiment is not a Windows implementation/source-code audit.
