# Release Lens

The release pipeline publishes one Apple-silicon DMG containing an ad-hoc-signed
Lens.app. The DMG is unsigned and neither artifact is Apple-notarized.
Paid Apple Developer enrollment and Apple signing secrets are not required.
Downloads remain restricted to readers of this private repository.

## Configure once

Create a GitHub App installed only on this repository with **Contents: write**,
**Pull requests: write**, **Administration: read**, and the implicit Metadata read permission. Do not
grant main-ruleset bypass. Set the repository variable `RELEASE_APP_ID` and
Actions secret `RELEASE_APP_PRIVATE_KEY`. The workflow creates a short-lived
installation token with only Contents and Pull requests write for PR and tag changes;
build jobs never receive it. A separate publication-policy job mints a token with
only Administration read to verify the immutability setting. That job has no
checkout and executes no repository code. The standard publisher token remains
Contents write. The [settings API requires admin read](https://docs.github.com/en/rest/repos/repos#check-if-immutable-releases-are-enabled-for-a-repository).

The App token is necessary for automatic PR CI and tag-triggered workflows:
a tag push using `GITHUB_TOKEN` does not trigger another workflow, and bot PR
events using that token can require approval. See
[GitHub's event rules](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow)
and [App tokens](https://github.com/actions/create-github-app-token).

Enable [immutable releases](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/establish-provenance-and-integrity/prevent-release-changes)
before the first publication. Set `RELEASE_PUBLISH_ENABLED=false` initially.
Missing or false means upload a verified draft and stop before publication.
The publisher rejects any other value except `true`.
Immutable releases protect published tags and assets; release notes remain editable.

Use squash merges with Conventional Commit titles. Keep the required
`code-quality` check and existing main ruleset enabled. Each Release PR is the
reviewed authorization to cut that version.

## Initial release

The implementation leaves `.release-please-manifest.json` empty: 0.1.0 has
not already shipped. After configuration, the next main push opens a separate
bootstrap Release PR with the initial changelog from
`.github/release-initial.md` and the manifest entry `".": "0.1.0"`.
If setup happens after the workflow's first skipped run, rerun that **Release PR**
workflow run on main; there is no unversioned manual publish command.

Review the current capability summary and known limitations. Merge the bootstrap
PR only when commissioning can proceed. Its exact merge commit is checked
against main, version mirrors and changelog, then receives an annotated
`v0.1.0` tag. The tag push runs complete portable, Linux common and macOS
verification; macOS builds and verifies the DMG. The publisher creates a draft
containing exactly the DMG and `SHA256SUMS`.

A maintainer with write access must download the draft assets **in a browser**.
Read-only testers cannot be assumed to see drafts. Use a clean test account or
device without prior development-build permissions. Record:

| Check         | Required evidence                                                                    |
| ------------- | ------------------------------------------------------------------------------------ |
| Source        | Tag, commit, workflow run/attempt, DMG SHA-256                                       |
| Packaging     | DMG verify, read-only mount, inner and copied app signature/metadata                 |
| Device        | Actual macOS version and Apple-silicon model                                         |
| First launch  | Install, Gatekeeper message, approved launch and menu bar                            |
| Permissions   | Accessibility and Screen Recording prompts and one Lens operation                    |
| Update        | Quit, replace app at the same path, launch, permission continuity or reauthorization |
| Reader access | A repository reader can download the published assets                                |

Test macOS 15.2 and the current macOS 26 family; record unavailable environments
as **not tested**. Static code-signing checks and hosted CI do not establish these
interactive outcomes. Do not add a quarantine-removal workaround until the Lens
failure is reproduced and the narrowly scoped remedy is verified.

After recording the results, set `RELEASE_PUBLISH_ENABLED=true` and rerun the
**all jobs of the same workflow run** so the independent publication-policy job
reads the current immutability setting. It reuses the original verified DMG
artifact ID and repeats code verification without compiling a new DMG. Check the published two assets and reader
access. Keep issue #18 open until this commissioning evidence is recorded.

## Subsequent releases

Ordinary main pushes update a single Release Please PR. Review its version and
`CHANGELOG.md`, wait for Code Quality, and squash-merge it. This automatically
creates the annotated tag and publishes after all release gates pass.

Version policy is explicit: fix/perf -> patch, feat -> minor, breaking changes
before 1.0 -> minor, breaking changes at or after 1.0 -> major. Non-user-facing
chore, ci, test, docs, refactor, build and style changes alone do not create a release.
Use `fix(deps)` or `feat(deps)` when a dependency change warrants distribution.
Changes anywhere in the repository, including shared packages, belong to the
one root component.

`apps/desktop/src-tauri/tauri.conf.json.version` is the authority. Root
`package.json`, desktop `package.json`, desktop `Cargo.toml`, the single
local desktop entry in `Cargo.lock`, and the root release manifest mirror it.
Shared packages and managed Agent runtimes keep independent versions.
Code Quality rejects inconsistent mirrors and malformed PR titles.

Release Please v4.4.1 bundles release-please 17.3.0. Its TOML updater requires
`$.package[?(@.name.value=="desktop")].version`: the parser wraps strings in
objects containing `value`. A conventional unwrapped JSONPath silently updates
nothing. The pinned Action conformance check executes the actual bundled
updaters and version strategy on copies, checks exact version-only changes, and
checks release/nonrelease commit classes. It downloads only checksum-pinned
Action code and templates; it does not install a second dependency tree.
Review its revision, file digests and ncc module IDs when upgrading the Action.
[Updater source](https://github.com/googleapis/release-please/blob/891bcf6253b390e39df9ff3e1c059a836bd39c98/src/updaters/generic-toml.ts).

Dependency admission retains the full observed graph and digest. Only the exact
local desktop application's validated version projects to an application-release
slot for admission. Every dependency version/source, feature and graph edge stays
literal. The reviewed schema-2 baselines cover both hosts and all seven variants;
there is no automatic baseline update during release.

## Failures and retries

The finite progression is:

`absent -> tagged -> validating -> building -> verified -> draft -> published`

Every nonterminal operation can fail. Published releases are terminal.
Tags are annotated JSON records of version, exact commit and originating PR.
Lightweight, malformed, conflicting, off-main and inconsistent tags fail.
A previous unfinished release blocks a new version. A delayed main event
reconciles at most one merged pending release PR; multiple candidates require
maintainer reconciliation. The current main tip never replaces the release
PR's exact merge commit as tag authority.

- Before a distribution artifact exists, rerun failed build/verification jobs.
- After upload, rerun failed jobs in the **same workflow run**. Successful
  prerequisite outputs retain the original frontend and distribution artifact IDs.
- Rerunning all jobs reuses the original distribution artifact and repeats
  portable/common/native code verification. It does not rebuild the DMG.
  Cached artifacts alone never substitute for complete verification.
- A partial release upload preserves matching assets and uploads only missing
  files. Different bytes, metadata, extra assets or incomplete uploads fail;
  the automation never deletes or overwrites conflicts.
- Original artifacts are retained for 30 days. An expired, missing-with-draft
  or conflicting artifact fails; recover deliberately using a new version.
  Do not move tags or overwrite published assets.
- If the publish response is lost, rerun the publisher: it verifies the existing
  terminal release and performs no additional writes.
- A general rerun of an already published tag ends at preflight without mutation.

The internal artifact has four files: the DMG, `SHA256SUMS`,
`release-manifest.json`, and `release-notes.md`. Only the first two are release
attachments. The manifest binds source, original run/attempt, configuration
digests, actual tool/SDK/image versions and asset sizes/hashes. Notes come from the
tagged changelog section and tagged install documentation, with a prior-tag link.
Uploaded bytes are downloaded and hashed before publishing.

No byte-for-byte reproducibility claim is made for separately rebuilt binaries
or DMGs. Recovery relies on preserving the original verified bytes.

## Local validation

Run from the repository root:

```sh
pnpm install --frozen-lockfile
mise run verify:portable
mise run verify:native
mise run verify:bundle -- dmg
```

The bundle task uses the prebuilt frontend and the admitted custom-protocol Rust
variant. It verifies DMG structure, Applications link, actual ad-hoc app signature,
copied-app signature, identifier, executable, name, both version fields, arm64 and
macOS 15.2 metadata. It does not use `spctl` or `stapler` acceptance as a gate.
The release wrapper configuration preserves ordinary app packaging.
[Tauri DMG](https://v2.tauri.app/distribute/dmg/) and
[Tauri ad-hoc signing](https://v2.tauri.app/distribute/sign/macos/#ad-hoc-signing).

Distribution credentials, provider authentication and interactive test results
are never inferred from a green build. Public downloads, Intel/universal,
automatic updates, PKG, Store submission and Apple notarization are separate work.
