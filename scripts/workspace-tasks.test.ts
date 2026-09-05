import { execFileSync, spawnSync } from "node:child_process";
import { appendFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = fileURLToPath(new URL("..", import.meta.url));
type Task = {
  name: string;
  source: string;
  depends: string[];
  wait_for: string[];
  depends_post: string[];
  dir: string;
  run: string[];
  sources: string[];
  outputs: string[];
};
const tasks: Task[] = JSON.parse(
  execFileSync("mise", ["tasks", "ls", "--json"], { cwd: root, encoding: "utf8" }),
);

// Replay the actual declared graph with inert leaf commands, never production effects.
function replay(entry: string, failure?: string) {
  const directory = mkdtempSync(join(tmpdir(), "lens-task-graph-"));
  try {
    const events = join(directory, "events.jsonl");
    appendFileSync(events, "");
    const probe = join(directory, "probe.cjs");
    writeFileSync(
      probe,
      'require("node:fs").appendFileSync(process.argv[2], JSON.stringify(process.argv[3]) + "\\n");' +
        "if (process.argv[3] === process.argv[4]) process.exit(7);",
    );
    const configuration = tasks
      .map((task) => {
        const command = [process.execPath, probe, events, task.name, failure ?? "none"]
          .map((argument) => `'${argument.replaceAll("'", "'\\''")}'`)
          .join(" ");
        return [
          `[tasks.${JSON.stringify(task.name)}]`,
          `dir = ${JSON.stringify(directory)}`,
          `depends = ${JSON.stringify(task.depends)}`,
          `wait_for = ${JSON.stringify(task.wait_for)}`,
          `depends_post = ${JSON.stringify(task.depends_post)}`,
          ...(task.run.length ? [`run = ${JSON.stringify(command)}`] : []),
        ].join("\n");
      })
      .join("\n\n");
    writeFileSync(join(directory, "mise.toml"), configuration);
    const result = spawnSync("mise", ["run", entry], {
      cwd: directory,
      env: { ...process.env, MISE_TRUSTED_CONFIG_PATHS: directory },
      encoding: "utf8",
      timeout: 10_000,
    });
    if (result.error) throw result.error;
    const completed: string[] = readFileSync(events, "utf8")
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line));
    return { status: result.status, completed, diagnostic: result.stderr };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe("repository task ownership", () => {
  it("keeps root scripts as single delegates and verification free of freshness skips", () => {
    const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    for (const script of Object.values(manifest.scripts) as string[]) {
      expect(script).toMatch(/^mise run [a-z:-]+(?: --)?$/u);
      expect(tasks.some((task) => script.split(" ")[2] === task.name)).toBe(true);
    }
    for (const task of tasks) {
      expect(task.source).toBe(resolve(root, "mise.toml"));
      expect([resolve(root), resolve(root, "apps/desktop")]).toContain(task.dir);
      expect(task.sources).toEqual([]);
      expect(task.outputs).toEqual([]);
      expect(task.depends_post).toEqual([]);
    }
  });

  it("runs native verification independently of JavaScript installation", () => {
    const result = replay("verify:native");
    expect(result).toMatchObject({ status: 0 });
    expect(result.completed.toSorted()).toEqual(
      ["check:dependencies", "check:rust", "rust:clippy", "rust:test"].toSorted(),
    );
    expect(result.completed.indexOf("check:rust")).toBeLessThan(
      result.completed.indexOf("rust:clippy"),
    );
    expect(result.completed.indexOf("rust:clippy")).toBeLessThan(
      result.completed.indexOf("rust:test"),
    );
  });

  it("finishes portable leaves before the full-verification Cargo writer chain", () => {
    const result = replay("verify");
    expect(result).toMatchObject({ status: 0 });
    const rust = result.completed.indexOf("check:rust");
    for (const leaf of [
      "check:identity",
      "check:terminology",
      "check:publication",
      "check:language",
      "check:quality",
      "check:icons",
      "check:types",
      "frontend:build",
      "test:repository",
      "test:frontend",
    ]) {
      expect(result.completed.indexOf(leaf)).toBeGreaterThanOrEqual(0);
      expect(result.completed.indexOf(leaf)).toBeLessThan(rust);
    }
    expect(new Set(result.completed).size).toBe(result.completed.length);
  });

  it("never starts the native writer after a portable prerequisite failure", () => {
    const result = replay("verify", "check:identity");
    expect(result.status).not.toBe(0);
    expect(result.completed).toContain("check:identity");
    expect(result.completed).not.toContain("check:rust");
    expect(result.completed).not.toContain("rust:clippy");
    expect(result.completed).not.toContain("rust:test");
  });

  it("does not continue to lint or test after normal Cargo checking fails", () => {
    const result = replay("verify:native", "check:rust");
    expect(result.status).not.toBe(0);
    expect(result.completed).toContain("check:rust");
    expect(result.completed).not.toContain("rust:clippy");
    expect(result.completed).not.toContain("rust:test");
  });
});
