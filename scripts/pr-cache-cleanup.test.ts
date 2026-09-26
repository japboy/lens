import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { parse } from "yaml";

const workflow = parse(readFileSync(".github/workflows/pr-cache-cleanup.yml", "utf8"));
const shell = workflow.jobs.cleanup.steps[0].run as string;
const repository = { id: 12, full_name: "owner/repository" };
const pr = { number: 129, state: "closed", base: { repo: repository } };
const closeEvent = { repository, action: "closed", pull_request: pr };
const completionEvent = {
  repository,
  action: "completed",
  workflow_run: { id: 456, name: "Code Quality", event: "pull_request", pull_requests: [pr] },
};
const ref = "refs/pull/129/merge";
const head = {
  repo: { id: 34, owner: { login: "contributor" } },
  ref: "feature/pr-cache",
  sha: "a".repeat(40),
};
const headPr = { ...pr, head };
const unassociatedCompletion = {
  ...completionEvent,
  workflow_run: {
    ...completionEvent.workflow_run,
    head_sha: head.sha,
    head_repository: head.repo,
    head_branch: head.ref,
    pull_requests: [],
  },
};

function execute({
  event = closeEvent,
  eventName = "pull_request_target",
  currentPr = pr,
  pages = [{ actions_caches: [{ id: 10, ref }] }],
  associatedPrPages = [[pr]],
  headPrPages = [[headPr]],
  fail = "",
  deleteError = "",
}: {
  event?: unknown;
  eventName?: string;
  currentPr?: unknown;
  pages?: unknown;
  associatedPrPages?: unknown;
  headPrPages?: unknown;
  fail?: string;
  deleteError?: string;
} = {}) {
  const directory = mkdtempSync(join(tmpdir(), "lens-pr-cache-cleanup-"));
  try {
    const eventPath = join(directory, "event.json");
    const fixturePath = join(directory, "fixture.json");
    const callsPath = join(directory, "calls.jsonl");
    writeFileSync(eventPath, JSON.stringify(event));
    writeFileSync(
      fixturePath,
      JSON.stringify({ currentPr, pages, associatedPrPages, headPrPages, fail, deleteError }),
    );
    writeFileSync(callsPath, "");
    // Execute the actual controller shell with a strict, recording GitHub API fake.
    writeFileSync(
      join(directory, "gh"),
      `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
const fixture = JSON.parse(fs.readFileSync(process.env.FIXTURE_PATH, 'utf8'));
fs.appendFileSync(process.env.CALLS_PATH, JSON.stringify(args) + '\\n');
if (args[0] !== 'api' || args[1] !== '--method') process.exit(90);
const endpoint = args[3];
const action = args[2] === 'DELETE' ? 'delete' : endpoint.includes('/commits/') ? 'associations' : endpoint.endsWith('/pulls') ? 'heads' : endpoint.includes('/pulls/') ? 'pr' : 'list';
if (fixture.fail === action) { process.stderr.write('API failed'); process.exit(1); }
if (action === 'delete' && fixture.deleteError) { process.stdout.write(fixture.deleteError); process.exit(1); }
if (action === 'pr') process.stdout.write(JSON.stringify(fixture.currentPr));
if (action === 'list') process.stdout.write(JSON.stringify(fixture.pages));
if (action === 'associations') process.stdout.write(JSON.stringify(fixture.associatedPrPages));
if (action === 'heads') process.stdout.write(JSON.stringify(fixture.headPrPages));
`,
      { mode: 0o755 },
    );
    const result = spawnSync("bash", ["-e", "-c", shell], {
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${directory}:${process.env.PATH}`,
        GH_REPO: repository.full_name,
        GITHUB_EVENT_NAME: eventName,
        GITHUB_EVENT_PATH: eventPath,
        FIXTURE_PATH: fixturePath,
        CALLS_PATH: callsPath,
      },
    });
    const calls = readFileSync(callsPath, "utf8")
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line) as string[]);
    return { ...result, calls, deletes: calls.filter((call) => call[2] === "DELETE") };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe("trusted pull request cache cleanup controller", () => {
  it("runs from trusted events without checkout or PR execution and uses separate per-PR concurrency", () => {
    expect(workflow.on).toEqual({
      pull_request_target: { types: ["closed"] },
      workflow_run: { workflows: ["Code Quality"], types: ["completed"] },
    });
    expect(workflow.permissions).toEqual({ actions: "write", "pull-requests": "read" });
    expect(workflow.concurrency).toEqual({
      group:
        "pr-cache-cleanup-${{ github.event.pull_request.number || github.event.workflow_run.pull_requests[0].number || github.event.workflow_run.id }}",
      "cancel-in-progress": false,
    });
    expect(workflow.jobs.cleanup.steps).toHaveLength(1);
    expect(workflow.jobs.cleanup.steps[0].uses).toBeUndefined();
    expect(workflow.jobs.cleanup.steps[0].env).toEqual({
      GH_TOKEN: "${{ github.token }}",
      GH_REPO: "${{ github.repository }}",
    });
    expect(shell).not.toContain("${{");
  });

  it("deletes every page by validated ID with an exact merge-ref API filter", () => {
    const result = execute({
      pages: [
        {
          actions_caches: [
            { id: 10, ref },
            { id: 20, ref },
          ],
        },
        { actions_caches: [{ id: 30, ref }] },
      ],
    });
    expect(result.status).toBe(0);
    expect(result.calls[1]).toEqual([
      "api",
      "--method",
      "GET",
      "repos/owner/repository/actions/caches",
      "--field",
      `ref=${ref}`,
      "--field",
      "per_page=100",
      "--paginate",
      "--slurp",
    ]);
    expect(result.deletes).toHaveLength(3);
    expect(result.deletes[2]).toEqual([
      "api",
      "--method",
      "DELETE",
      "repos/owner/repository/actions/caches/30",
      "--include",
    ]);
  });

  it("rechecks closure after verification completes, including a later rerun", () => {
    for (const event of [closeEvent, completionEvent]) {
      const eventName = event === closeEvent ? "pull_request_target" : "workflow_run";
      const closed = execute({ event, eventName });
      expect(closed.status).toBe(0);
      expect(closed.deletes).toHaveLength(1);
      const reopened = execute({ event, eventName, currentPr: { ...pr, state: "open" } });
      expect(reopened.status).toBe(0);
      expect(reopened.calls).toHaveLength(1);
      expect(reopened.deletes).toEqual([]);
    }
  });

  it("accepts no caches and deduplicates repeated IDs", () => {
    expect(execute({ pages: [{ actions_caches: [] }] }).status).toBe(0);
    const result = execute({
      pages: [
        {
          actions_caches: [
            { id: 10, ref },
            { id: 10, ref },
          ],
        },
      ],
    });
    expect(result.status).toBe(0);
    expect(result.deletes).toHaveLength(1);
  });

  it.each(["refs/heads/main", "refs/pull/130/merge", "refs/pull/129/head"])(
    "never deletes any ID if a later page contains %s",
    (unsafeRef) => {
      const result = execute({
        pages: [
          { actions_caches: [{ id: 10, ref }] },
          { actions_caches: [{ id: 20, ref: unsafeRef }] },
        ],
      });
      expect(result.status).not.toBe(0);
      expect(result.deletes).toEqual([]);
    },
  );

  it.each([0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, "10", "10; touch /tmp/unexpected"])(
    "rejects malformed cache ID %s before mutations",
    (id) => {
      const result = execute({ pages: [{ actions_caches: [{ id, ref }] }] });
      expect(result.status).not.toBe(0);
      expect(result.deletes).toEqual([]);
    },
  );

  it("fails before API calls for missing, foreign or injected associations", () => {
    for (const pullRequests of [
      [],
      [{ ...pr, number: "129; touch /tmp/unexpected" }],
      [{ ...pr, base: { repo: { id: 13 } } }],
    ]) {
      const result = execute({
        eventName: "workflow_run",
        event: {
          ...completionEvent,
          workflow_run: { ...completionEvent.workflow_run, pull_requests: pullRequests },
        },
      });
      expect(result.status).not.toBe(0);
      expect(result.calls).toEqual([]);
    }
  });

  it("resolves missing closed PR associations from the validated run commit across all pages", () => {
    const result = execute({
      eventName: "workflow_run",
      event: {
        ...completionEvent,
        workflow_run: {
          ...completionEvent.workflow_run,
          head_sha: "a".repeat(40),
          pull_requests: [],
        },
      },
      associatedPrPages: [[], [pr]],
    });
    expect(result.status).toBe(0);
    expect(result.calls[0]).toEqual([
      "api",
      "--method",
      "GET",
      `repos/owner/repository/commits/${"a".repeat(40)}/pulls`,
      "--field",
      "per_page=100",
      "--paginate",
      "--slurp",
    ]);
    expect(result.deletes).toHaveLength(1);
  });

  it("rejects unavailable or foreign commit associations without deleting caches", () => {
    for (const associatedPrPages of [
      [],
      [[]],
      {},
      [null],
      [[{ ...pr, base: { repo: { id: 13 } } }]],
    ]) {
      const result = execute({
        eventName: "workflow_run",
        event: {
          ...completionEvent,
          workflow_run: {
            ...completionEvent.workflow_run,
            head_sha: "a".repeat(40),
            pull_requests: [],
          },
        },
        associatedPrPages,
      });
      expect(result.status).not.toBe(0);
      expect(result.deletes).toEqual([]);
    }
  });

  it("accepts duplicate server associations but validates all associations before mutation", () => {
    const event = {
      ...completionEvent,
      workflow_run: { ...completionEvent.workflow_run, pull_requests: [pr, pr] },
    };
    expect(execute({ eventName: "workflow_run", event }).deletes).toHaveLength(1);
    const invalid = execute({
      eventName: "workflow_run",
      event: {
        ...event,
        workflow_run: { ...event.workflow_run, pull_requests: [pr, { ...pr, number: -1 }] },
      },
    });
    expect(invalid.status).not.toBe(0);
    expect(invalid.calls).toEqual([]);
  });

  it("recovers an unmerged closed PR only with the exact run repository, branch and SHA", () => {
    const result = execute({
      eventName: "workflow_run",
      event: unassociatedCompletion,
      associatedPrPages: [[]],
      headPrPages: [[], [headPr]],
      currentPr: headPr,
    });
    expect(result.status).toBe(0);
    expect(result.calls[1]).toEqual([
      "api",
      "--method",
      "GET",
      "repos/owner/repository/pulls",
      "--field",
      "state=all",
      "--raw-field",
      "head=contributor:feature/pr-cache",
      "--field",
      "per_page=100",
      "--paginate",
      "--slurp",
    ]);
    expect(result.deletes).toHaveLength(1);
  });

  it("rejects reused branches, other forks, mismatched refs and foreign base repositories", () => {
    for (const candidate of [
      { ...headPr, head: { ...head, sha: "b".repeat(40) } },
      { ...headPr, head: { ...head, repo: { ...head.repo, id: 35 } } },
      { ...headPr, head: { ...head, ref: "other-branch" } },
      { ...headPr, base: { repo: { id: 13 } } },
    ]) {
      const result = execute({
        eventName: "workflow_run",
        event: unassociatedCompletion,
        associatedPrPages: [[]],
        headPrPages: [[candidate]],
        currentPr: headPr,
      });
      expect(result.status).not.toBe(0);
      expect(result.deletes).toEqual([]);
      expect(result.calls).toHaveLength(2);
    }
  });

  it("does not use ambiguous head filtering when the exact run identity is absent", () => {
    for (const overrides of [
      { head_sha: "main" },
      { head_repository: null },
      { head_repository: { ...head.repo, id: "34" } },
      { head_repository: { ...head.repo, owner: { login: "owner:other" } } },
      { head_branch: "" },
      { head_branch: "branch\ncontrol" },
    ]) {
      const result = execute({
        eventName: "workflow_run",
        event: {
          ...unassociatedCompletion,
          workflow_run: { ...unassociatedCompletion.workflow_run, ...overrides },
        },
        associatedPrPages: [[]],
      });
      expect(result.status).not.toBe(0);
      expect(result.deletes).toEqual([]);
      expect(
        result.calls.some((call) => call[3]?.endsWith("/pulls") && !call[3]?.includes("/commits/")),
      ).toBe(false);
    }
  });

  it("fails closed on missing/malformed head results or a head changed after lookup", () => {
    for (const options of [
      { headPrPages: [] },
      { headPrPages: [[]] },
      { headPrPages: {} },
      { headPrPages: [null] },
      { currentPr: { ...headPr, head: { ...head, sha: "b".repeat(40) } } },
      { fail: "heads" },
    ]) {
      const result = execute({
        eventName: "workflow_run",
        event: unassociatedCompletion,
        associatedPrPages: [[]],
        currentPr: headPr,
        ...options,
      });
      expect(result.status).not.toBe(0);
      expect(result.deletes).toEqual([]);
    }
  });

  it("accepts only an actual HTTP 404 after a concurrent deletion", () => {
    expect(execute({ deleteError: "HTTP/2.0 404 Not Found\r\n\r\n{}" }).status).toBe(0);
    for (const deleteError of [
      "HTTP/2.0 403 Forbidden",
      "HTTP/2.0 500 Server Error",
      '{"message":"404"}',
    ])
      expect(execute({ deleteError }).status).not.toBe(0);
  });

  it("rejects unrelated events, repositories and workflows", () => {
    for (const options of [
      { eventName: "push" },
      { event: { ...closeEvent, action: "opened" } },
      { event: { ...closeEvent, repository: { ...repository, full_name: "other/repo" } } },
      {
        eventName: "workflow_run",
        event: {
          ...completionEvent,
          workflow_run: { ...completionEvent.workflow_run, event: "push" },
        },
      },
      {
        eventName: "workflow_run",
        event: {
          ...completionEvent,
          workflow_run: { ...completionEvent.workflow_run, name: "Other Workflow" },
        },
      },
    ]) {
      const result = execute(options);
      expect(result.status).not.toBe(0);
      expect(result.calls).toEqual([]);
    }
  });

  it("fails closed on malformed current state or cache pagination", () => {
    for (const options of [
      { currentPr: { ...pr, number: 130 } },
      { currentPr: { ...pr, state: "unknown" } },
      { currentPr: { ...pr, base: { repo: { id: 13 } } } },
      { pages: [] },
      { pages: {} },
      { pages: [{ actions_caches: null }] },
    ]) {
      const result = execute(options);
      expect(result.status).not.toBe(0);
      expect(result.deletes).toEqual([]);
    }
  });

  it.each(["pr", "list", "delete"])("surfaces %s API failures", (fail) => {
    const result = execute({ fail });
    expect(result.status).not.toBe(0);
    expect(result.deletes).toHaveLength(fail === "delete" ? 1 : 0);
  });
});
