import { appendFileSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { github } from "./github.ts";
import { control, previousRelease, validateReleasePr } from "./control.ts";
import type { PullRequest } from "./control.ts";
import { assertPrTitle, changelogSection, cleanSource, git, inspectTag } from "./source.ts";
import {
  packageArtifact,
  sha256,
  verifyArtifact,
  CONFIGURATION_FILES,
  releaseNotes,
} from "./artifact.ts";
import { publish, requireReleaseJobs } from "./publish.ts";
import { preflight } from "./preflight.ts";

const root = fileURLToPath(new URL("../../", import.meta.url));
const env = (name: string) => {
  const value = process.env[name];
  if (!value) throw new Error(`Required explicit input: ${name}`);
  return value;
};
const output = (name: string, value: string) => {
  if (/[\r\n]/u.test(value)) throw new Error("Multiline workflow output rejected");
  appendFileSync(env("GITHUB_OUTPUT"), `${name}=${value}\n`);
};
const mode = process.argv[2];
if (process.argv.length !== 3) throw new Error("Exactly one release operation required");
if (mode === "pr-title") {
  assertPrTitle(env("PR_TITLE"));
} else if (mode === "control") {
  const api = github(env("GH_TOKEN"), env("GITHUB_REPOSITORY"));
  const result = await control(api.request, root, env("SOURCE_SHA"), env("GITHUB_REPOSITORY"));
  output("state", result);
} else if (mode === "preflight") {
  const api = github(env("GH_TOKEN"), env("GITHUB_REPOSITORY"));
  const tag = env("RELEASE_TAG");
  const data = inspectTag(root, tag);
  validateReleasePr(
    await api.request<PullRequest>(`/pulls/${data.pullRequest}`),
    data.commit,
    env("GITHUB_REPOSITORY"),
  );
  changelogSection(git(root, "show", `${data.commit}:CHANGELOG.md`), data.version);
  output("source_sha", data.commit);
  const previous = await previousRelease(api.request, data.version);
  const plan = await preflight(api.request, tag, data.commit, env("GITHUB_RUN_ID"));
  output("state", plan.state);
  output("artifact_id", plan.artifactId);
  output("previous_tag", previous ?? "");
} else if (mode === "package") {
  packageArtifact(root, join(root, "target/release-artifact"), {
    tag: env("RELEASE_TAG"),
    source: env("SOURCE_SHA"),
    repository: env("GITHUB_REPOSITORY"),
    runId: env("GITHUB_RUN_ID"),
    runAttempt: env("GITHUB_RUN_ATTEMPT"),
    previousTag: process.env.PREVIOUS_TAG || null,
  });
} else if (mode === "publish") {
  const api = github(env("GH_TOKEN"), env("GITHUB_REPOSITORY"));
  const tag = env("RELEASE_TAG");
  const data = inspectTag(root, tag);
  cleanSource(root, data.commit);
  validateReleasePr(
    await api.request<PullRequest>(`/pulls/${data.pullRequest}`),
    data.commit,
    env("GITHUB_REPOSITORY"),
  );
  await previousRelease(api.request, data.version);
  requireReleaseJobs(env("PORTABLE_RESULT"), env("COMMON_RESULT"), env("NATIVE_RESULT"));
  const expected = {
    tag,
    source: data.commit,
    repository: env("GITHUB_REPOSITORY"),
    runId: env("GITHUB_RUN_ID"),
  };
  const directory = join(root, "target/release-artifact");
  const manifest = verifyArtifact(directory, expected);
  for (const path of CONFIGURATION_FILES)
    if (manifest.configuration[path] !== sha256(readFileSync(join(root, path))))
      throw new Error("Artifact configuration differs from tagged source");
  const section = changelogSection(git(root, "show", `${data.commit}:CHANGELOG.md`), data.version);
  const previous = await previousRelease(api.request, data.version);
  if (
    manifest.previousTag !== (previous ?? null) ||
    readFileSync(join(directory, "release-notes.md"), "utf8") !==
      releaseNotes(section, manifest, manifest.assets[0]!)
  )
    throw new Error("Release notes differ from the tagged source");
  const enabled = process.env.RELEASE_PUBLISH_ENABLED ?? "";
  if (!["", "false", "true"].includes(enabled)) throw new Error("Invalid publication switch");
  process.stdout.write(
    `${await publish(api, directory, expected, enabled === "true", process.env.IMMUTABILITY_RESULT === "success")}\n`,
  );
} else {
  throw new Error("Unknown release operation");
}
