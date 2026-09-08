import { existsSync, readFileSync } from "node:fs";
import { dirname } from "node:path";
import { describe, expect, it, vi } from "vitest";
import { actionRevision, withActionSource } from "./action-source.ts";

const sha = "a".repeat(40);
const workflow = (uses: string) => `jobs:\n  release:\n    steps:\n      - uses: ${uses}\n`;

describe("workflow-owned Action revision", () => {
  it("uses the executable step, ignoring comments and run text", () => {
    expect(
      actionRevision(
        `# uses: googleapis/release-please-action@v4\n${workflow(`'googleapis/release-please-action@${sha}' # current`)}      - run: |\n          uses: googleapis/release-please-action@v3\n`,
      ),
    ).toBe(sha);
  });
  it("follows a workflow-only revision change", () => {
    expect(actionRevision(workflow(`googleapis/release-please-action@${"b".repeat(40)}`))).toBe(
      "b".repeat(40),
    );
  });
  it.each(["v5", "main", "a".repeat(7), `${sha}/dist`, "${{ inputs.ref }}"])(
    "rejects a non-full-SHA reference: %s",
    (ref) => {
      expect(() => actionRevision(workflow(`googleapis/release-please-action@${ref}`))).toThrow(
        "full commit SHA",
      );
    },
  );
  it("rejects missing, duplicate and malformed declarations", () => {
    expect(() => actionRevision(workflow("actions/checkout@v4"))).toThrow("exactly one");
    expect(() =>
      actionRevision(
        `${workflow(`googleapis/release-please-action@${sha}`)}      - uses: googleapis/release-please-action@${sha}\n`,
      ),
    ).toThrow("exactly one");
    expect(() => actionRevision("jobs: {}\njobs: {}\n")).toThrow(/.+/u);
  });
});

describe("same-revision temporary Action source", () => {
  it("fetches bundle and templates for each run and cleans up after verification", async () => {
    const directories: string[] = [];
    const retrieve = vi.fn<typeof fetch>(async () => new Response("source bytes"));
    for (let i = 0; i < 2; i++) {
      await withActionSource(
        sha,
        ["header.hbs"],
        async ({ directory, bundle }) => {
          directories.push(directory);
          expect(dirname(bundle)).toBe(directory);
          expect(readFileSync(bundle, "utf8")).toBe("source bytes");
          expect(readFileSync(`${directory}/header.hbs`, "utf8")).toBe("source bytes");
        },
        retrieve,
      );
    }
    expect(directories[0]).not.toBe(directories[1]);
    for (const directory of directories) expect(existsSync(directory)).toBe(false);
    expect(retrieve.mock.calls.map(([url]) => url)).toEqual(
      Array.from({ length: 2 }, () => [
        `https://raw.githubusercontent.com/googleapis/release-please-action/${sha}/dist/index.js`,
        `https://raw.githubusercontent.com/googleapis/release-please-action/${sha}/dist/header.hbs`,
      ]).flat(),
    );
  });
  it("cleans up when verification rejects the bundle", async () => {
    let path = "";
    await expect(
      withActionSource(
        sha,
        [],
        async ({ directory }) => {
          path = directory;
          throw new Error("unsupported bundle");
        },
        async () => new Response("unsupported"),
      ),
    ).rejects.toThrow("unsupported bundle");
    expect(existsSync(path)).toBe(false);
  });
  it("does not verify incomplete downloads or accept unsafe template paths", async () => {
    const verify = vi.fn<() => Promise<undefined>>(async () => undefined);
    await expect(
      withActionSource(
        sha,
        ["header.hbs"],
        verify,
        async () => new Response("missing", { status: 404 }),
      ),
    ).rejects.toThrow("Could not retrieve");
    expect(verify).not.toHaveBeenCalled();
    await expect(withActionSource(sha, ["../header.hbs"], verify)).rejects.toThrow(/.+/u);
  });
});
