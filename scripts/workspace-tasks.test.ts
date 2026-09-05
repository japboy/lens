import { execFileSync, spawnSync } from "node:child_process";
import {
  appendFileSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
  globSync,
  statSync,
} from "node:fs";
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
  file?: string;
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
          ...(task.run.length || task.file ? [`run = ${JSON.stringify(command)}`] : []),
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
  it("owns commands only in mise and keeps verification free of freshness skips", () => {
    const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    expect(manifest.scripts).toBeUndefined();
    expect(tasks).toHaveLength(39);
    expect(new Set(tasks.map((task) => task.name)).size).toBe(tasks.length);
    for (const task of tasks) {
      expect(task.source).toBe(
        task.file
          ? resolve(root, "mise-tasks", `${task.name.replaceAll(":", "/")}.ts`)
          : resolve(root, "mise.toml"),
      );
      expect([resolve(root), resolve(root, "apps/desktop")]).toContain(task.dir);
      expect(task.sources).toEqual([]);
      expect(task.outputs).toEqual([]);
      expect(task.depends_post).toEqual([]);
    }
    const fileTasks = tasks.filter((task) => task.file);
    expect(fileTasks).toHaveLength(11);
    for (const task of fileTasks) expect(task.run).toEqual([]);
  });

  it("does not discover adjacent non-executable test modules as tasks", () => {
    const testFiles = [...globSync("mise-tasks/**/*.test.ts", { cwd: root })];
    expect(testFiles).toHaveLength(6);
    for (const file of testFiles) {
      expect(statSync(resolve(root, file)).mode & 0o111).toBe(0);
      expect(tasks.map((task) => task.source)).not.toContain(resolve(root, file));
    }
  });

  it("discovers and runs root file tasks from the desktop working directory", () => {
    expect(
      execFileSync("mise", ["run", "check:identity"], {
        cwd: resolve(root, "apps/desktop"),
        encoding: "utf8",
      }),
    ).toContain("Product identity policy passed");
  });

  it("runs native verification independently of JavaScript installation", () => {
    const result = replay("verify:native");
    expect(result).toMatchObject({ status: 0 });
    expect(result.completed.toSorted()).toEqual(
      [
        "check:dependencies",
        "check:rust",
        "rust:clippy",
        "rust:test",
        "check:rust:release",
      ].toSorted(),
    );
    expect(result.completed.indexOf("check:rust")).toBeLessThan(
      result.completed.indexOf("rust:clippy"),
    );
    expect(result.completed.indexOf("rust:clippy")).toBeLessThan(
      result.completed.indexOf("rust:test"),
    );
    expect(result.completed.indexOf("rust:test")).toBeLessThan(
      result.completed.indexOf("check:rust:release"),
    );
  });

  it("runs the complete Linux chain without selecting a native host task", () => {
    const result = replay("verify:linux");
    expect(result.status).toBe(0);
    const chain = [
      "check:rust:linux",
      "rust:clippy:linux",
      "rust:test:linux",
      "check:rust:apple",
      "check:rust:native-features",
    ];
    for (const task of chain) {
      expect(result.completed).toContain(task);
    }
    for (const [index, task] of chain.slice(1).entries())
      expect(result.completed.indexOf(chain[index]!)).toBeLessThan(result.completed.indexOf(task));
    expect(result.completed).not.toContain("check:rust");
    expect(result.completed.indexOf("frontend:build")).toBeLessThan(
      result.completed.indexOf(chain[0]!),
    );
    expect(new Set(result.completed).size).toBe(result.completed.length);
  });

  it("does not attempt feature compatibility after a Linux normal-check failure", () => {
    const result = replay("verify:linux", "check:rust:linux");
    expect(result.status).not.toBe(0);
    expect(result.completed).not.toContain("rust:test:linux");
    expect(result.completed).not.toContain("check:rust:native-features");
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
      "check:boundaries",
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
