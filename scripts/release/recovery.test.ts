import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { GitHub, Request } from "./github.ts";
import type { AdmittedRelease } from "./admission.ts";
import { requireMergedReleasePr } from "./admission.ts";
import type { PullRequest, Release } from "./control.ts";
import { sha256 } from "./artifact.ts";
import { RELEASE_WORKFLOW, RECEIPT_NAME, verifyArtifactV2, promoteArtifactV2 } from "./receipt.ts";
import type { ArtifactManifestV2 } from "./receipt.ts";
import { resumeRelease, verifyBuildProvenance, verifyPublishedRelease } from "./resume.ts";
import type { OriginalArtifact, RemoteAsset } from "./resume.ts";
import { publishReleaseV2 } from "./publisher.ts";

const source = "a".repeat(40),
  controller = "b".repeat(40);
const admitted: AdmittedRelease = {
  tag: "v0.1.0",
  version: "0.1.0",
  source,
  pullRequest: 18,
  releaseId: 50,
  legacy: false,
  draft: true,
};
function fixture() {
  const directory = mkdtempSync(join(tmpdir(), "lens-recovery-"));
  const dmg = Buffer.from("already validated native DMG fixture");
  const name = "Lens_0.1.0_aarch64.dmg";
  const sums = Buffer.from(`${sha256(dmg)}  ${name}\n`);
  const notes = "Installation guidance preserved separately from mutable release metadata";
  const manifest: ArtifactManifestV2 = {
    schema: 2,
    repository: "owner/repo",
    tag: admitted.tag,
    source,
    controller,
    workflow: RELEASE_WORKFLOW,
    runId: "123",
    runAttempt: "2",
    verificationAttempt: "2",
    version: "0.1.0",
    previousTag: null,
    assets: [
      { name, size: dmg.length, sha256: sha256(dmg) },
      { name: "SHA256SUMS", size: sums.length, sha256: sha256(sums) },
    ],
    notesSha256: sha256(notes),
    configuration: {},
    tools: {},
    applicationSignature: "adhoc",
    dmgSignature: "unsigned",
    notarization: "not-performed",
  };
  writeFileSync(join(directory, name), dmg);
  writeFileSync(join(directory, "SHA256SUMS"), sums);
  writeFileSync(join(directory, "release-notes.md"), notes);
  writeFileSync(join(directory, "release-manifest.json"), JSON.stringify(manifest));
  const release: Release = {
    id: 50,
    tag_name: admitted.tag,
    target_commitish: source,
    draft: true,
    prerelease: false,
    name: "v0.1.0",
    body: "Release Please owns this changelog",
    upload_url: "https://uploads.github.test",
    immutable: false,
  };
  const artifact: OriginalArtifact = {
    id: 456,
    name: `release-${admitted.tag}`,
    expired: false,
    workflow_run: { id: 123, head_sha: controller, head_branch: "main" },
  };
  const run = {
    id: 123,
    event: "workflow_dispatch",
    head_sha: controller,
    head_branch: "main",
    path: RELEASE_WORKFLOW,
    run_attempt: 2,
    repository: { full_name: "owner/repo" },
    head_repository: { full_name: "owner/repo" },
  };
  const verification = { name: "release / release-verification", conclusion: "success" };
  const remote: (RemoteAsset & { bytes: Buffer })[] = [];
  const writes: string[] = [];
  const reads: string[] = [];
  const flags = {
    originalMissing: false,
    failUpload: "",
    losePublication: false,
    forbidActions: false,
    changedSource: false,
  };
  const request = (async (path: string, method = "GET", payload?: unknown) => {
    reads.push(path);
    if (flags.forbidActions && path.startsWith("/actions/"))
      throw new Error("Actions must not be consulted for terminal verification");
    if (method !== "GET") writes.push(`${method} ${path}`);
    if (path.startsWith("/releases?")) return [release];
    if (path.startsWith("/releases/50/assets")) return remote;
    if (path.startsWith("/actions/artifacts?"))
      return { artifacts: flags.originalMissing ? [] : [artifact] };
    if (path === "/actions/artifacts/456") return artifact;
    if (path.includes("/jobs?")) return { jobs: [verification] };
    if (path === "/actions/runs/123/attempts/2") return run;
    if (path === "/actions/runs/123/attempts/1") return { ...run, run_attempt: 1 };
    if (path === "/releases/50" && method === "PATCH") {
      if (JSON.stringify(payload) !== JSON.stringify({ draft: false, make_latest: "true" }))
        throw new Error("Unexpected publication fields");
      release.draft = false;
      release.immutable = true;
      if (flags.losePublication) throw new Error("Publication response lost");
      return release;
    }
    throw new Error(`Unexpected request: ${method} ${path}`);
  }) as Request;
  const api: GitHub = {
    request,
    upload: async (_url, assetName, bytes) => {
      writes.push(`upload ${assetName}`);
      if (flags.failUpload === assetName) throw new Error("Injected upload failure");
      remote.push({
        id: remote.length + 1,
        name: assetName,
        size: bytes.length,
        state: "uploaded",
        digest: `sha256:${sha256(bytes)}`,
        bytes,
      });
    },
    download: async (path) => {
      const id = Number(path.split("/").at(-1));
      const asset = remote.find((entry) => entry.id === id);
      if (!asset) throw new Error("Unknown remote asset");
      return asset.bytes;
    },
  };
  const provenance = {
    repository: "owner/repo",
    artifactId: "456",
    readmit: async () => ({ ...admitted, source: flags.changedSource ? controller : source }),
    admitController: (sha: string) => {
      expect(sha).toBe(controller);
    },
  };
  const publish = (enabled = true, immutable = true) =>
    publishReleaseV2(api, directory, admitted, provenance, enabled, immutable);
  return {
    directory,
    manifest,
    release,
    artifact,
    run,
    verification,
    remote,
    writes,
    reads,
    flags,
    api,
    provenance,
    publish,
    close: () => rmSync(directory, { recursive: true, force: true }),
  };
}

describe("standard source ownership", () => {
  it("admits old/new owned heads without trusting labels and rejects forks/base/source changes", () => {
    const pr: PullRequest = {
      number: 18,
      merged: true,
      state: "closed",
      merge_commit_sha: source,
      base: { ref: "main", repo: { full_name: "owner/repo" } },
      head: {
        ref: "release-please--branches--main--components--lens",
        repo: { full_name: "owner/repo" },
      },
      labels: [],
    };
    requireMergedReleasePr(pr, source, "owner/repo");
    requireMergedReleasePr(
      { ...pr, head: { ...pr.head, ref: "release-please--branches--main" } },
      source,
      "owner/repo",
    );
    for (const bad of [
      { ...pr, merged: false },
      { ...pr, merge_commit_sha: controller },
      { ...pr, base: { ...pr.base, ref: "other" } },
      { ...pr, head: { ...pr.head, ref: "feature" } },
      { ...pr, head: { ...pr.head, repo: { full_name: "fork/repo" } } },
    ])
      expect(() => requireMergedReleasePr(bad, source, "owner/repo")).toThrow(/.+/u);
  });
});

describe("schema 2 artifact and Actions provenance", () => {
  it("accepts an original artifact from a different controller run with distinct product/controller SHAs", async () => {
    const f = fixture();
    try {
      expect(
        verifyArtifactV2(f.directory, { ...admitted, repository: "owner/repo" }).controller,
      ).toBe(controller);
      await verifyBuildProvenance(f.api.request, f.manifest, "456", f.provenance);
      expect(await resumeRelease(f.api, admitted, "owner/repo")).toEqual({
        state: "reuse",
        artifactId: "456",
        runId: "123",
      });
    } finally {
      f.close();
    }
  });
  it.each([
    "expired",
    "controller",
    "branch",
    "repository",
    "event",
    "workflow",
    "attempt",
    "verification",
  ])("rejects %s provenance conflicts before upload", async (fault) => {
    const f = fixture();
    try {
      if (fault === "expired") f.artifact.expired = true;
      if (fault === "controller") f.run.head_sha = source;
      if (fault === "branch") f.run.head_branch = "feature";
      if (fault === "repository") f.run.head_repository.full_name = "fork/repo";
      if (fault === "event") f.run.event = "pull_request_target";
      if (fault === "workflow") f.run.path = ".github/workflows/other.yml";
      if (fault === "attempt") f.run.run_attempt = 3;
      if (fault === "verification") f.verification.conclusion = "skipped";
      await expect(f.publish()).rejects.toThrow(/.+/u);
      expect(f.writes).toEqual([]);
    } finally {
      f.close();
    }
  });
  it("promotes an earlier native build after a successful later verification attempt", async () => {
    const f = fixture();
    try {
      f.manifest.runAttempt = "1";
      f.manifest.verificationAttempt = "1";
      writeFileSync(join(f.directory, "release-manifest.json"), JSON.stringify(f.manifest));
      const promoted = promoteArtifactV2(f.directory, f.manifest, "2");
      expect(promoted.runAttempt).toBe("1");
      expect(promoted.verificationAttempt).toBe("2");
      const request = f.api.request;
      f.api.request = (async (path: string, method?: string, body?: unknown) => {
        if (path.includes("/attempts/1/jobs?"))
          throw new Error("The failed first aggregate must not authorize publication");
        return request(path, method, body);
      }) as Request;
      expect(await f.publish()).toBe("published");
      expect(f.reads).toContain("/actions/runs/123/attempts/1");
      expect(f.reads).toContain("/actions/runs/123/attempts/2/jobs?per_page=100&page=1");
    } finally {
      f.close();
    }
  });
  it("retains gate attempt 1 when only promotion is retried in attempt 2", async () => {
    const f = fixture();
    try {
      f.manifest.runAttempt = "1";
      f.manifest.verificationAttempt = "1";
      writeFileSync(join(f.directory, "release-manifest.json"), JSON.stringify(f.manifest));
      // The gate's retained output is 1; the promotion job's current attempt is 2.
      const promoted = promoteArtifactV2(f.directory, f.manifest, "1");
      expect(promoted.verificationAttempt).toBe("1");
      const request = f.api.request;
      f.api.request = (async (path: string, method?: string, body?: unknown) => {
        if (path.includes("/attempts/2"))
          throw new Error("No verification gate ran in promotion retry attempt 2");
        return request(path, method, body);
      }) as Request;
      expect(await f.publish()).toBe("published");
      expect(f.reads).toContain("/actions/runs/123/attempts/1/jobs?per_page=100&page=1");
    } finally {
      f.close();
    }
  });
  it.each(["run", "controller", "earlier", "missing"])(
    "rejects %s promotion/verification identity",
    (fault) => {
      const f = fixture();
      try {
        const expected = {
          ...f.manifest,
          ...(fault === "run" ? { runId: "999" } : {}),
          ...(fault === "controller" ? { controller: source } : {}),
        };
        expect(() =>
          promoteArtifactV2(
            f.directory,
            expected,
            fault === "earlier" ? "1" : fault === "missing" ? "" : "2",
          ),
        ).toThrow(/.+/u);
      } finally {
        f.close();
      }
    },
  );
  it("rejects tampered local artifact bytes", () => {
    const f = fixture();
    try {
      writeFileSync(join(f.directory, "SHA256SUMS"), "wrong");
      expect(() =>
        verifyArtifactV2(f.directory, { ...admitted, repository: "owner/repo" }),
      ).toThrow(/.+/u);
    } finally {
      f.close();
    }
  });
});

describe("draft and published finite recovery", () => {
  it("builds only an empty draft without a retained original; refuses rebuilding uploaded bytes", async () => {
    const f = fixture();
    try {
      f.flags.originalMissing = true;
      expect(await resumeRelease(f.api, admitted, "owner/repo")).toEqual({ state: "build" });
      f.remote.push({
        id: 9,
        name: "SHA256SUMS",
        size: 1,
        state: "uploaded",
        bytes: Buffer.from("x"),
      });
      await expect(resumeRelease(f.api, admitted, "owner/repo")).rejects.toThrow(
        "without the original artifact",
      );
    } finally {
      f.close();
    }
  });
  it("uploads exactly DMG/checksum/receipt, preserves RP metadata and verifies terminal state without Actions", async () => {
    const f = fixture();
    try {
      expect(await f.publish()).toBe("published");
      expect(f.remote.map((asset) => asset.name)).toEqual([
        "Lens_0.1.0_aarch64.dmg",
        "SHA256SUMS",
        RECEIPT_NAME,
      ]);
      expect(f.release.name).toBe("v0.1.0");
      expect(f.release.body).toBe("Release Please owns this changelog");
      f.flags.forbidActions = true;
      f.artifact.expired = true;
      const count = f.writes.length;
      expect((await resumeRelease(f.api, admitted, "owner/repo")).state).toBe("published");
      expect(await f.publish()).toBe("already-published");
      expect(f.writes).toHaveLength(count);
    } finally {
      f.close();
    }
  });
  it("resumes a matching partial upload without overwriting existing bytes", async () => {
    const f = fixture();
    try {
      f.flags.failUpload = "SHA256SUMS";
      await expect(f.publish()).rejects.toThrow("upload failure");
      f.flags.failUpload = "";
      await f.publish();
      expect(f.writes.filter((entry) => entry === "upload Lens_0.1.0_aarch64.dmg")).toHaveLength(1);
    } finally {
      f.close();
    }
  });
  it("rejects conflicting existing bytes before adding anything", async () => {
    const f = fixture();
    try {
      const bytes = Buffer.from("conflict");
      f.remote.push({ id: 1, name: "SHA256SUMS", bytes, size: bytes.length, state: "uploaded" });
      await expect(f.publish()).rejects.toThrow("conflict");
      expect(f.writes).toEqual([]);
    } finally {
      f.close();
    }
  });
  it("recovers a lost publication response via immutable assets without original artifacts", async () => {
    const f = fixture();
    try {
      f.flags.losePublication = true;
      await expect(f.publish()).rejects.toThrow("response lost");
      f.flags.forbidActions = true;
      expect(await f.publish()).toBe("already-published");
    } finally {
      f.close();
    }
  });
  it("never publishes without immutable policy and rejects mutable terminal state", async () => {
    const f = fixture();
    try {
      await expect(f.publish(true, false)).rejects.toThrow("immutability");
      expect(f.release.draft).toBe(true);
      f.release.draft = false;
      await expect(verifyPublishedRelease(f.api, admitted, "owner/repo")).rejects.toThrow(
        "immutable",
      );
    } finally {
      f.close();
    }
  });
  it("detects remote corruption and receipt identity substitution", async () => {
    const f = fixture();
    try {
      await f.publish();
      const receipt = f.remote.find((asset) => asset.name === RECEIPT_NAME)!;
      const parsed = JSON.parse(receipt.bytes.toString("utf8"));
      parsed.source = controller;
      receipt.bytes = Buffer.from(JSON.stringify(parsed));
      receipt.size = receipt.bytes.length;
      receipt.digest = `sha256:${sha256(receipt.bytes)}`;
      await expect(verifyPublishedRelease(f.api, admitted, "owner/repo")).rejects.toThrow(
        "identity mismatch",
      );
    } finally {
      f.close();
    }
  });
  it("reports legacy published without retroactively claiming receipt verification", async () => {
    const f = fixture();
    try {
      f.release.draft = false;
      f.flags.forbidActions = true;
      expect(await resumeRelease(f.api, { ...admitted, legacy: true }, "owner/repo")).toEqual({
        state: "legacy-published",
      });
      await expect(
        publishReleaseV2(
          f.api,
          f.directory,
          { ...admitted, legacy: true },
          f.provenance,
          true,
          true,
        ),
      ).rejects.toThrow("Legacy");
      expect(f.writes).toEqual([]);
    } finally {
      f.close();
    }
  });
  it("readmits tag/source immediately before publishing and stops on movement during upload", async () => {
    const f = fixture();
    try {
      const upload = f.api.upload;
      f.api.upload = async (...args) => {
        await upload(...args);
        f.flags.changedSource = true;
      };
      await expect(f.publish()).rejects.toThrow("admission changed");
      expect(f.release.draft).toBe(true);
      expect(f.writes.some((entry) => entry.startsWith("PATCH"))).toBe(false);
    } finally {
      f.close();
    }
  });
  it("writes a source-bound receipt with actual original artifact/run identities", async () => {
    const f = fixture();
    try {
      expect(await f.publish(false)).toBe("draft");
      const receipt = JSON.parse(
        f.remote.find((asset) => asset.name === RECEIPT_NAME)!.bytes.toString("utf8"),
      );
      expect(receipt).toMatchObject({
        schema: 2,
        releaseId: 50,
        pullRequest: 18,
        source,
        controller,
        runId: "123",
        runAttempt: "2",
        artifactId: "456",
      });
      expect(readFileSync(join(f.directory, "release-manifest.json"), "utf8")).not.toContain(
        '"artifactId"',
      );
    } finally {
      f.close();
    }
  });
});
