# Reviewed historical checkpoint: v0.5.0

This fixed record authorizes verification of one preserved release inventory after
its original Actions artifact became unavailable. Approval of this checked-in
record is an explicit trust decision for this historical exception. It is not a
configurable allowlist, reconstruction of the original artifact, or an attestation
of a build performed before this recovery mechanism existed.

The release assets and their hashes were saved and checked before the full rerun:
release 392344988, source/controller a9b261e084fd2f96fc8745c725525d17a7e56d91,
merged release PR 104, run 35496202983 attempt 1, artifact 10601590488.
The original log records artifact upload at 2026-09-20 07:21:37 UTC and successful
download at 07:21:54 UTC. The ZIP digest is retained as historical evidence only;
the missing ZIP and its unpublished manifest/notes cannot be recovered from the
three release files. Attempt 2 observed the missing original at 07:24:04 UTC.
The separately saved before-rerun release metadata and local copies of all three
assets supplied the pinned IDs, byte lengths, and SHA-256 digests in this record.

Verification requires the exact repository, tag, source, PR and release identity;
the original lightweight tag; the exact three original asset IDs and metadata;
downloaded hashes; the schema 2 receipt and checksum contents; and the historical
attempt 1 run identity and successful aggregate verification job 106040694548.
Another version is outside the exception. A missing receipt, partial upload,
replaced asset, changed byte, missing historical run, or failed gate is rejected.
Already published state additionally requires immutable=true. The module performs
only reads. The caller still owns publication policy and final source readmission.
No remote release or GitHub permissions are changed by adding this record.

Tests use a synthetic distribution and a mocked fixed-record file, with real
SHA-256 calculations. Production record construction separately hashed the saved
original files and matched the before-rerun API digest/size fields. Live API
validation is separate from these deterministic tests.

Evidence endpoints:

- https://github.com/japboy/lens/actions/runs/35496202983/attempts/1
- https://github.com/japboy/lens/pull/104
- https://api.github.com/repos/japboy/lens/releases/392344988
- https://docs.github.com/en/rest/actions/workflow-jobs#get-a-job-for-a-workflow-run
- https://docs.github.com/en/rest/releases/assets#get-a-release-asset
