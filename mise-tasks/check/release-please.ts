#!/usr/bin/env node
//MISE description = "Verify the exact release Action updaters and version policy without installing its dependency tree"
//MISE dir = "{{config_root}}"

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { runInThisContext } from "node:vm";
import { actionRevision, withActionSource } from "../../scripts/release/action-source.ts";
import { VERSION_FILES, versionState } from "../../scripts/release/version.ts";

const root = fileURLToPath(new URL("../../", import.meta.url));
const revision = actionRevision(
  readFileSync(join(root, ".github/workflows/release-please.yml"), "utf8"),
);
const templates: string[] = JSON.parse(
  readFileSync(join(root, "scripts/release/please-templates.json"), "utf8"),
);
await withActionSource(revision, templates, async ({ directory, bundle }) => {
  const original = readFileSync(bundle, "utf8");
  // Expose the pinned ncc modules in a test-only copy. Disable the Action entry point;
  // the updater/strategy implementations are unmodified and receive no GitHub client.
  const entry = "if (require.main === require.cache[eval('__filename')]) {";
  const exportsLine = "module.exports = __webpack_exports__;";
  assert.equal(original.split(entry).length, 2);
  assert.equal(original.split(exportsLine).length, 2);
  const adapted = original.replace(entry, "if (false) {").replace(
    exportsLine,
    `module.exports = {
  library: __nccwpck_require__(24363),
  factory: __nccwpck_require__(75695),
  Merge: __nccwpck_require__(90514).Merge,
  parseCommits: __nccwpck_require__(69158).parseConventionalCommits,
  Version: __nccwpck_require__(17348).Version
};`,
  );
  const module = { exports: {} };
  const wrapper = runInThisContext(
    `(function(require, module, exports, __filename, __dirname) { ${adapted}\n})`,
    { filename: bundle },
  ) as (
    require: NodeJS.Require,
    module: { exports: unknown },
    exports: unknown,
    filename: string,
    dirname: string,
  ) => void;
  wrapper(createRequire(import.meta.url), module, module.exports, bundle, directory);
  type Version = { toString(): string };
  type ReleaseProposal = {
    headRefName: string;
    title: { toString(): string };
    body: { toString(): string };
    labels: string[];
    updates: Updater[];
  };
  type Updater = {
    path: string;
    createIfMissing?: boolean;
    updater: { updateContent(content: string): string };
  };
  type Strategy = {
    buildReleasePullRequest(
      commits: { sha: string; message: string; files: string[] }[],
      latest?: unknown,
    ): Promise<{ version: Version; updates: Updater[] } | undefined>;
    extraFileUpdates(
      version: Version,
      versions: Map<string, Version>,
      format: string,
    ): Promise<Updater[]>;
    buildUpdates(options: {
      newVersion: Version;
      changelogEntry: string;
      versionsMap: Map<string, Version>;
    }): Promise<Updater[]>;
  };
  const bundled = module.exports as {
    library: {
      VERSION: string;
      setLogger(logger: unknown): void;
      Manifest: {
        fromManifest(
          github: unknown,
          branch: string,
          configFile: string,
          manifestFile: string,
          options: unknown,
        ): Promise<{
          buildPullRequests(): Promise<ReleaseProposal[]>;
          createPullRequests(): Promise<unknown[]>;
        }>;
      };
    };
    Merge: new (
      github: unknown,
      branch: string,
      config: unknown,
      options: unknown,
    ) => {
      run(
        candidates: unknown[],
      ): Promise<{ pullRequest: { headRefName: string; title: { toString(): string } } }[]>;
    };
    parseCommits(
      commits: { sha: string; message: string; files: string[] }[],
    ): { sha: string; message: string; files: string[] }[];
    factory: { buildStrategy(options: Record<string, unknown>): Promise<Strategy> };
    Version: { parse(value: string): Version };
  };

  const quiet = {
    info() {},
    warn(message: string) {
      throw new Error(message);
    },
    debug() {},
    error(message: string) {
      throw new Error(message);
    },
  };
  bundled.library.setLogger(quiet);
  const config = JSON.parse(readFileSync(join(root, "release-please-config.json"), "utf8"));
  assert.deepEqual(Object.keys(config.packages), ["."]);
  assert.equal(
    config["separate-pull-requests"],
    false,
    "One grouped branch must match tag admission",
  );
  const component = config.packages["."];
  const strategy = await bundled.factory.buildStrategy({
    github: { repository: { owner: "fixture", repo: "lens" } },
    targetBranch: "main",
    path: ".",
    releaseType: config["release-type"],
    packageName: component["package-name"],
    includeComponentInTag: config["include-component-in-tag"],
    includeVInTag: config["include-v-in-tag"],
    initialVersion: component["initial-version"],
    bumpMinorPreMajor: config["bump-minor-pre-major"],
    bumpPatchForMinorPreMajor: config["bump-patch-for-minor-pre-major"],
    extraFiles: component["extra-files"],
    changelogSections: component["changelog-sections"],
  });
  const current = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
  let cases = 0;
  for (const version of ["0.1.1", "0.2.0", "1.0.0"]) {
    const parsed = bundled.Version.parse(version);
    const updates = [
      ...(await strategy.buildUpdates({
        newVersion: parsed,
        changelogEntry: "fixture",
        versionsMap: new Map(),
      })),
      ...(await strategy.extraFileUpdates(parsed, new Map(), "YYYY-MM-DD")),
    ];
    for (const path of [
      "package.json",
      ...component["extra-files"].map((file: { path: string }) => file.path),
    ]) {
      const matching = updates.filter((update) => update.path === path);
      assert.equal(matching.length, 1, `Exactly one updater for ${path}`);
      const content = readFileSync(join(root, path), "utf8");
      const expected =
        path === "Cargo.lock"
          ? content.replace(
              `name = "desktop"\nversion = "${current}"`,
              `name = "desktop"\nversion = "${version}"`,
            )
          : content
              .replace(`"version": "${current}"`, `"version": "${version}"`)
              .replace(`version = "${current}"`, `version = "${version}"`);
      assert.equal(
        matching[0]!.updater.updateContent(content),
        expected,
        `Exact version-only update of ${path}`,
      );
      cases++;
    }
  }
  for (const [previous, message, expected] of [
    [undefined, "feat: initial", "0.1.0"],
    ["0.1.0", "fix: repair", "0.1.1"],
    ["0.1.0", "feat: new capability", "0.2.0"],
    ["0.1.0", "feat!: incompatible capability", "0.2.0"],
    ["1.0.0", "feat!: incompatible capability", "2.0.0"],
    ["0.1.0", "perf: improve", "0.1.1"],
    ["0.1.0", "fix(deps): update dependency", "0.1.1"],
    ...["chore", "docs", "ci", "test", "refactor", "build", "style"].map((type) => [
      "0.1.0",
      `${type}: maintenance`,
      undefined,
    ]),
  ] as const) {
    const latest = previous
      ? {
          tag: { version: bundled.Version.parse(previous), toString: () => `v${previous}` },
          sha: "a".repeat(40),
          notes: "",
        }
      : undefined;
    const result = await strategy.buildReleasePullRequest(
      bundled.parseCommits([
        { sha: "b".repeat(40), message: message!, files: ["packages/usecase/src/lib.rs"] },
      ]),
      latest,
    );
    assert.equal(result?.version.toString(), expected, `Version policy: ${message}`);
    cases++;
  }
  const candidate = await strategy.buildReleasePullRequest(
    bundled.parseCommits([
      {
        sha: "c".repeat(40),
        message: "feat: shared capability",
        files: ["packages/domain/src/lib.rs"],
      },
    ]),
  );
  const grouped = await new bundled.Merge(
    { repository: { owner: "fixture", repo: "lens" } },
    "main",
    { ".": { releaseType: "node", separatePullRequests: false } },
    {
      pullRequestTitlePattern: config["group-pull-request-title-pattern"],
      pullRequestHeader: config["pull-request-header"],
      pullRequestFooter: config["pull-request-footer"],
    },
  ).run([
    {
      path: ".",
      config: { releaseType: "node", separatePullRequests: false },
      pullRequest: candidate,
    },
  ]);
  assert.equal(grouped.length, 1);
  assert.equal(grouped[0]!.pullRequest.headRefName, "release-please--branches--main");
  assert.equal(grouped[0]!.pullRequest.title.toString(), "chore(main): release 0.1.0");
  cases++;
  // Exercise the real Manifest path, including JSON config parsing, the empty version
  // manifest, complete initial history, grouping, changelog and manifest updaters.
  assert.equal(config["bootstrap-sha"], undefined);
  assert.equal(config["pull-request-header"], undefined);
  assert.ok(!config["pull-request-footer"].includes("— Codex"));
  for (const message of ["feat: first supported capability", "docs: maintenance"]) {
    let existing:
      | { number: number; headBranchName: string; body: string; labels: string[] }
      | undefined;
    let maintenanceMerged = false;
    let updateCount = 0;
    const github = {
      repository: { owner: "fixture", repo: "lens" },
      async getFileJson(path: string) {
        assert.ok(["release-please-config.json", ".release-please-manifest.json"].includes(path));
        return path === "release-please-config.json" ? config : {};
      },
      async *releaseIterator() {
        yield* [];
      },
      async *tagIterator() {
        yield* [];
      },
      async *mergeCommitIterator() {
        if (maintenanceMerged)
          yield {
            sha: "f".repeat(40),
            message: "test: isolate release fixtures",
            files: ["scripts/release/control.test.ts"],
          };
        yield { sha: "d".repeat(40), message, files: ["packages/domain/src/lib.rs"] };
        yield {
          sha: "e".repeat(40),
          message: message.startsWith("feat:")
            ? "feat: oldest supported capability"
            : "docs: oldest maintenance",
          files: ["README.md"],
        };
      },
      async *pullRequestIterator(branch: string, state: string) {
        assert.equal(branch, "main");
        if (state === "OPEN" && existing) yield existing;
      },
      async updatePullRequest(number: number, proposal: ReleaseProposal, branch: string) {
        assert.equal(number, existing!.number);
        assert.equal(branch, "main");
        assert.equal(proposal.headRefName, existing!.headBranchName);
        assert.equal(proposal.body.toString(), existing!.body);
        updateCount++;
        return existing;
      },
    };
    const manifest = await bundled.library.Manifest.fromManifest(
      github,
      "main",
      "release-please-config.json",
      ".release-please-manifest.json",
      {
        logger: {
          ...quiet,
          warn(message: string) {
            assert.match(
              message,
              /^(?:Expected 1 releases, only found 0|Missing 1 paths: \.|No version for path \.|No latest release pull request found\.)$/u,
            );
          },
        },
      },
    );
    const proposals = await manifest.buildPullRequests();
    assert.equal(proposals.length, message.startsWith("feat:") ? 1 : 0);
    if (proposals.length) {
      const proposal = proposals[0]!;
      assert.equal(proposal.headRefName, "release-please--branches--main");
      assert.equal(proposal.title.toString(), "chore(main): release 0.1.0");
      assert.deepEqual(proposal.labels, ["autorelease: pending"]);
      const updates = new Map(proposal.updates.map((update) => [update.path, update]));
      assert.equal(updates.size, proposal.updates.length);
      const applicable = [...updates.values()].filter(
        (update) => update.createIfMissing || existsSync(join(root, update.path)),
      );
      assert.deepEqual(
        applicable.map((update) => update.path).sort(),
        [
          "CHANGELOG.md",
          ".release-please-manifest.json",
          "package.json",
          ...component["extra-files"].map((file: { path: string }) => file.path),
        ].sort(),
      );
      const files = Object.fromEntries(
        VERSION_FILES.map((path) => [
          path,
          updates
            .get(path)!
            .updater.updateContent(
              path === ".release-please-manifest.json"
                ? "{}"
                : readFileSync(join(root, path), "utf8"),
            ),
        ]),
      );
      assert.deepEqual(versionState(files), { version: "0.1.0", bootstrapped: true });
      const changelog = updates.get("CHANGELOG.md")!.updater.updateContent("");
      assert.match(changelog, /## .*0\.1\.0/u);
      assert.ok(changelog.includes("first supported capability"));
      assert.ok(changelog.includes("oldest supported capability"));
      assert.deepEqual(
        JSON.parse(updates.get(".release-please-manifest.json")!.updater.updateContent("{}")),
        { ".": "0.1.0" },
      );
      existing = {
        number: 53,
        headBranchName: proposal.headRefName,
        body: proposal.body.toString(),
        labels: proposal.labels,
      };
      maintenanceMerged = true;
      assert.equal((await manifest.buildPullRequests())[0]!.body.toString(), existing.body);
      assert.equal((await manifest.createPullRequests()).length, 1);
      assert.equal(updateCount, 1, "Update the existing release PR even when only tests changed");
      cases++;
    }
    cases++;
  }
  process.stdout.write(
    `Release Please Action ${revision} (release-please ${bundled.library.VERSION}) conformance passed: ${cases} cases.\n`,
  );
});
