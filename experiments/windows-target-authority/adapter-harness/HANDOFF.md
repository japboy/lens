# Windows 11 isolated adapter verification

This is an experiment, not a product adapter. Windows remains `Unsupported`.
Only the supplied controlled fixture windows are in scope. Do not point the
runner at a user window or send its output to an external Agent.

## Synchronize safely

The PR handoff must supply the full 40-character source SHA after push. Replace
`<PR_HANDOFF_SHA>` below with that exact value; do not substitute a moving branch.
Use a clean dedicated checkout. Preserve any local work before switching commits.

```powershell
git fetch origin codex/windows-common-contracts
git switch --detach <PR_HANDOFF_SHA>
git rev-parse HEAD
git status --porcelain --untracked-files=all
```

Prerequisites: Windows 11 x64 interactive desktop, PowerShell 7, Node.js with
`node:test` support, Git, and an already installed x64 MSVC developer environment
with the Windows SDK. Enter that developer environment in PowerShell 7 before
running. No installation or privilege elevation is performed by the collector.
Use an ordinary desktop session, not a locked/service/headless session. Record
any capture/provider denial as evidence; do not silently alter machine policy.

```powershell
pwsh -NoProfile -File ./experiments/windows-target-authority/adapter-harness/collect-evidence.ps1 `
  -ExpectedSourceSha <PR_HANDOFF_SHA> `
  -OutputDirectory C:\lens-evidence\pr68-<SHORT_SHA>-run1
```

The destination must not exist and must be outside the checkout. The script checks
the exact SHA and clean worktree, runs all synthetic experiment tests, builds the
three native groups, then runs the adapter matrix. A failed build stops the native
matrix. Individual case failures are retained and later cases still run. No existing
directory is deleted, and no process outside the runner's owned children is killed.
Every recorded command has a 180-second external deadline, a 16 MiB limit per
stdout/stderr stream, and a further five-second close-confirmation bound. On
abnormal termination only that command's owned process tree is targeted. This
outer bound is distinct from the runner's shorter operation deadline; an outer
timeout is a harness failure, not a successful native timeout experiment.

## Read the result, not just the exit code

Each runner stdout is a JSON record with `stage`, `reason`, `cleanup`, `accepted`,
`rejected`, `published`, transitions, native observation and provenance. A nonzero
exit must be investigated. Exit zero alone is insufficient: compare actual reason,
publication and cleanup against the case below. Unexpected timeout in an ordinary
case is not success even when the runner successfully enforced its deadline.
Require `verification: "observed"`, `inputsUnchanged: true`, and `cleanup: "closed"`
as well as the expected scenario reason. Native child records retain bounded raw
stdout/stderr, PID and observed close/exit details; absent data remains unavailable.

| Cases                                            | Required interpretation                                                                                                                                                                        |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| acquisition `ordinary`, provider `release`       | Completed observation, one matched candidate published, confirmed closed cleanup. Native API failure/timeout is not ordinary success.                                                          |
| acquisition `stop-before-*`                      | Cancelled before the selected next operation; no publication or automatic rebind; confirmed cleanup.                                                                                           |
| acquisition `replace-before-*`, `mismatched-pid` | Authority uncertainty, no publication or subsequent reacquisition; confirmed cleanup. This is rejection, not a proof that HWND/runtime IDs cannot be reused.                                   |
| provider `stop-inflight`                         | Provider entry observed before Stop, admission terminated before release, returned late candidate rejected, no publication; confirmed cleanup. Without provider entry the overlap is untested. |
| provider `provider-deadline`                     | Provider entry/hold overlaps deadline, timed out without publication, cleanup recorded independently. Missing overlap makes the intended scenario inconclusive.                                |

Classify every case explicitly as successful observation, expected rejection,
failure, inconclusive, or not executed. `termination-unconfirmed` must never be
reported as `closed`. Retain unexpected results instead of repeating until green.

Synthetic tests cover wrong/old/duplicate identities, malformed/oversized/truncated
output, startup stalls and unconfirmed cleanup using controlled test doubles.
Do not label those as native Windows API observations. The native matrix exercises
the real fixture/probe programs, but parent-generated operation envelopes are channel
correlation, not native identity echoes or a lifetime guarantee. Existing WGC
center-pixel observations do not validate encoded images or structured extraction.

## Submit evidence

Keep the complete new directory. It contains source SHA/clean-state/environment,
source hashes, original build manifests and binaries, each command's arguments,
stdout/stderr, exit code and elapsed time, runner JSON, and before/after snapshots
of named experiment processes. `checksums.json` hashes every other retained file;
it does not authenticate the evidence's origin. Process snapshots are independent
point-in-time observations and cannot prove historical absence between samples.
Child stream data is available only to the extent actually retained by each native
record; missing child stdout/stderr must be marked unavailable, never reconstructed.

Review the evidence for incidental machine paths before sharing. Archive the whole
directory without editing its contents, submit it with a per-case classification,
and mention any interrupted run, missing stream, residual process or environmental
change. Product composition, real-user capture, merge and release remain out of scope.
