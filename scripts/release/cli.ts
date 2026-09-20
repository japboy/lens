import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { CONFIGURATION_FILES, sha256, releaseNotes } from "./artifact.ts";
import { fileURLToPath } from "node:url";
import { join, resolve } from "node:path";
import { github } from "./github.ts";
import { previousRelease } from "./control.ts";
import { assertPrTitle, changelogSection, commitSha, git, requireMain } from "./source.ts";
import { admitRelease } from "./admission.ts";
import { resumeRelease } from "./resume.ts";
import {
  packageArtifactV2,
  promoteArtifactV2,
  RELEASE_WORKFLOW,
  verifyArtifactV2,
} from "./receipt.ts";
import {
  publishDurableRelease,
  publishReviewedCheckpoint,
  restoreDurableArtifact,
} from "./durable.ts";
import { BUNDLE_NAME, createBundle } from "./recovery-bundle.ts";
import { verifyReviewedCheckpoint } from "./checkpoint.ts";

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
function controller(): string {
  const sha = commitSha(env("CONTROLLER_SHA"));
  if (
    sha !== env("GITHUB_SHA") ||
    env("GITHUB_REF") !== "refs/heads/main" ||
    !["push", "workflow_dispatch"].includes(env("GITHUB_EVENT_NAME")) ||
    git(root, "rev-parse", "HEAD") !== sha
  )
    throw new Error("Release controller must be the trusted main workflow revision");
  requireMain(root, sha);
  return sha;
}
const mode = process.argv[2];
if (process.argv.length !== 3) throw new Error("Exactly one release operation required");
if (mode === "pr-title") {
  assertPrTitle(env("PR_TITLE"));
} else if (mode === "generate") {
  const { generate } = await import("./generate.ts");
  const policyToken = env("GH_POLICY_TOKEN");
  delete process.env.GH_POLICY_TOKEN;
  const result = await generate(
    root,
    env("GH_TOKEN"),
    policyToken,
    env("GITHUB_REPOSITORY"),
    controller(),
    process.env.RELEASE_TAG || "",
  );
  output("tag", result.state === "release" ? result.tag : "");
} else if (mode === "delta") {
  const { verifyReleaseDelta } = await import("./release-delta.ts");
  verifyReleaseDelta(root, env("BASE_SHA"), env("HEAD_SHA"));
} else if (mode === "preflight") {
  controller();
  const api = github(env("GH_TOKEN"), env("GITHUB_REPOSITORY"));
  const admitted = await admitRelease(
    api.request,
    root,
    env("GITHUB_REPOSITORY"),
    env("RELEASE_TAG"),
  );
  const plan = await resumeRelease(api, admitted, env("GITHUB_REPOSITORY"));
  const previous = admitted.draft
    ? await previousRelease(api.request, admitted.version)
    : undefined;
  output("source_sha", admitted.source);
  output("state", plan.state);
  output("artifact_id", plan.state === "reuse" ? plan.artifactId : "");
  output("artifact_run_id", plan.state === "reuse" ? plan.runId : "");
  output("previous_tag", previous ?? "");
} else if (mode === "promote") {
  const sha = controller();
  const manifest = promoteArtifactV2(
    join(root, "target/release-artifact"),
    {
      repository: env("GITHUB_REPOSITORY"),
      tag: env("RELEASE_TAG"),
      source: env("SOURCE_SHA"),
      controller: sha,
      runId: env("GITHUB_RUN_ID"),
    },
    env("VERIFICATION_ATTEMPT"),
  );
  writeFileSync(
    join(root, "target/release-artifact", BUNDLE_NAME),
    createBundle(join(root, "target/release-artifact"), manifest, env("GITHUB_RUN_ATTEMPT")),
  );
} else if (mode === "package") {
  if (
    git(root, "rev-parse", "HEAD") !== commitSha(env("CONTROLLER_SHA")) ||
    env("CONTROLLER_SHA") !== env("GITHUB_SHA")
  )
    throw new Error("Packaging controller differs from workflow revision");
  const sourceRoot = resolve(env("SOURCE_ROOT"));
  packageArtifactV2(sourceRoot, join(sourceRoot, "target/release-artifact"), {
    tag: env("RELEASE_TAG"),
    source: env("SOURCE_SHA"),
    controller: commitSha(env("CONTROLLER_SHA")),
    repository: env("GITHUB_REPOSITORY"),
    runId: env("GITHUB_RUN_ID"),
    runAttempt: env("GITHUB_RUN_ATTEMPT"),
    workflow: RELEASE_WORKFLOW,
    previousTag: process.env.PREVIOUS_TAG || null,
  });
} else if (mode === "publish") {
  controller();
  const api = github(env("GH_TOKEN"), env("GITHUB_REPOSITORY"));
  const admitted = await admitRelease(
    api.request,
    root,
    env("GITHUB_REPOSITORY"),
    env("RELEASE_TAG"),
  );
  const enabled = process.env.RELEASE_PUBLISH_ENABLED ?? "";
  if (!["", "false", "true"].includes(enabled)) throw new Error("Invalid publication switch");
  const directory = join(root, "target/release-artifact");
  const policy = {
    repository: env("GITHUB_REPOSITORY"),
    readmit: () => admitRelease(api.request, root, env("GITHUB_REPOSITORY"), env("RELEASE_TAG")),
    admitController(sha: string) {
      requireMain(root, commitSha(sha));
      if (!git(root, "show", `${sha}:scripts/release/receipt.ts`).includes(RELEASE_WORKFLOW))
        throw new Error("Original controller predates the admitted build contract");
    },
  };
  const checkpoint = await verifyReviewedCheckpoint(api, admitted, policy.repository);
  if (admitted.draft && !checkpoint && process.env.RECOVERY_STATE === "durable")
    await restoreDurableArtifact(api, directory, admitted, policy);
  if (admitted.draft && !checkpoint) {
    const manifest = verifyArtifactV2(
      directory,
      {
        ...admitted,
        repository: env("GITHUB_REPOSITORY"),
      },
      true,
    );
    for (const path of CONFIGURATION_FILES) {
      const sourceBytes = execFileSync("git", ["show", `${admitted.source}:${path}`], {
        cwd: root,
      });
      if (manifest.configuration[path] !== sha256(sourceBytes))
        throw new Error("Artifact configuration differs from admitted source");
    }
    const previous = await previousRelease(api.request, admitted.version);
    const section = changelogSection(
      git(root, "show", `${admitted.source}:CHANGELOG.md`),
      admitted.version,
    );
    if (
      manifest.previousTag !== (previous ?? null) ||
      readFileSync(join(directory, "release-notes.md"), "utf8") !==
        releaseNotes(section, manifest, manifest.assets[0]!)
    )
      throw new Error("Artifact notes differ from admitted source");
  }
  const result = checkpoint
    ? await publishReviewedCheckpoint(
        api,
        admitted,
        policy,
        enabled === "true",
        process.env.IMMUTABILITY_RESULT === "success",
      )
    : await publishDurableRelease(
        api,
        directory,
        admitted,
        policy,
        enabled === "true",
        process.env.IMMUTABILITY_RESULT === "success",
      );
  process.stdout.write(`${result}\n`);
} else {
  throw new Error("Unknown release operation");
}
