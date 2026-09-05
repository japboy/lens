import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { runInThisContext } from "node:vm";

const root = fileURLToPath(new URL("../../", import.meta.url));
const revision = "5c625bfb5d1ff62eadeeb3772007f7f66fdcf071";
const digest = "04c6aa6cdc67f4f1be03de390c78b5558a461ab56a855dfd1b5bb15a70a11199";
const destination = join(root, "target/release-please-conformance/index.cjs");
const workflow = readFileSync(join(root, ".github/workflows/release-please.yml"), "utf8");
assert.ok(
  workflow.includes(`googleapis/release-please-action@${revision}`),
  "Review conformance pin with the Action revision",
);
if (!existsSync(destination)) {
  const response = await fetch(
    `https://raw.githubusercontent.com/googleapis/release-please-action/${revision}/dist/index.js`,
    { signal: AbortSignal.timeout(60_000) },
  );
  assert.ok(response.ok, "Could not retrieve the pinned Action bundle");
  mkdirSync(join(root, "target/release-please-conformance"), { recursive: true });
  writeFileSync(destination, Buffer.from(await response.arrayBuffer()));
}
const templates: Record<string, string> = JSON.parse(
  readFileSync(join(root, "scripts/release/please-templates.json"), "utf8"),
);
for (const [name, expected] of Object.entries(templates)) {
  assert.match(name, /^[a-z]+[0-9]?\.hbs$/u);
  const path = join(root, "target/release-please-conformance", name);
  if (!existsSync(path)) {
    const response = await fetch(
      `https://raw.githubusercontent.com/googleapis/release-please-action/${revision}/dist/${name}`,
      { signal: AbortSignal.timeout(60_000) },
    );
    assert.ok(response.ok, `Could not retrieve ${name}`);
    writeFileSync(path, Buffer.from(await response.arrayBuffer()));
  }
  assert.equal(
    createHash("sha256").update(readFileSync(path)).digest("hex"),
    expected,
    `Template integrity: ${name}`,
  );
}
const original = readFileSync(destination, "utf8");
assert.equal(
  createHash("sha256").update(original).digest("hex"),
  digest,
  "Pinned Action bundle integrity",
);
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
  { filename: destination },
) as (
  require: NodeJS.Require,
  module: { exports: unknown },
  exports: unknown,
  filename: string,
  dirname: string,
) => void;
wrapper(
  createRequire(import.meta.url),
  module,
  module.exports,
  destination,
  join(root, "target/release-please-conformance"),
);
type Version = { toString(): string };
type Updater = { path: string; updater: { updateContent(content: string): string } };
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
  library: { VERSION: string; setLogger(logger: unknown): void };
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
assert.equal(bundled.library.VERSION, "17.3.0");
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
process.stdout.write(`Pinned Release Please Action conformance passed: ${cases} cases.\n`);
