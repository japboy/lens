import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  assertGraphMatches,
  graphArguments,
  graphDigest,
  parseFeatureGraph,
} from "./workspace-feature-graphs.ts";
import { BUILD_VARIANTS } from "./workspace-policy.ts";

describe("target-specific transitive feature resolution", () => {
  it("preserves duplicate package feature instances and dependency edges", () => {
    const graph = parseFeatureGraph(
      "0consumer v0.1.0 (/fixture/consumer)|\n1shared v1.0.0|normal\n1shared v1.0.0|build\n2derive v1.0.0 (proc-macro)|\n",
      "/fixture",
    );
    expect(graph.nodes.filter((node) => node.name === "shared")).toHaveLength(2);
    expect(graph.edges).toHaveLength(3);
    expect(graph.nodes[graph.roots[0]!]!.source).toBe("path:consumer");
    expect(() => assertGraphMatches(graph, structuredClone(graph))).not.toThrow();
    const changed = structuredClone(graph);
    changed.nodes[0]!.features.push("native");
    expect(() => assertGraphMatches(graph, changed)).toThrow("Dependency/feature graph changed");
    expect(graphDigest(graph)).not.toBe(graphDigest(changed));
  });

  it.each([
    "",
    "unexpected output",
    "1shared v1.0.0|",
    "0shared v1.0.0|a,a",
    "0local v1.0.0 (/outside)|",
    "0local v1.0.0 (git+https://example.test/repo)|",
    "0shared v1.0.0| (*)",
  ])("rejects incomplete/unsupported graph input: %s", (output) => {
    expect(() => parseFeatureGraph(output, "/fixture")).toThrow(
      /Empty|Unsupported|Incomplete|Duplicate|escapes/u,
    );
  });

  it("selects development dependencies only for explicit test variants", () => {
    for (const variant of BUILD_VARIANTS) {
      const args = graphArguments(variant);
      expect(args[args.indexOf("--edges") + 1]).toBe(
        variant.operation === "test" ? "normal,build,dev" : "normal,build",
      );
      expect(args).toContain(variant.target);
      expect(args).not.toContain("--all-features");
    }
  });

  it("detects a real native-unified dependency API change that a producer-only check misses", () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "lens-feature-boundary-")));
    const run = (args: string[]) =>
      spawnSync("cargo", args, {
        cwd: root,
        encoding: "utf8",
        timeout: 15_000,
        env: { ...process.env, CARGO_TARGET_DIR: join(root, "target") },
      });
    try {
      writeFileSync(
        join(root, "Cargo.toml"),
        '[workspace]\nmembers = ["shared", "consumer", "shell"]\nresolver = "2"\n',
      );
      for (const name of ["shared", "consumer", "shell"])
        mkdirSync(join(root, name, "src"), { recursive: true });
      writeFileSync(
        join(root, "shared/Cargo.toml"),
        '[package]\nname = "shared"\nversion = "0.1.0"\nedition = "2021"\n[features]\nnative = []\n',
      );
      writeFileSync(
        join(root, "shared/src/lib.rs"),
        '#[cfg(not(feature = "native"))]\npub fn value() -> u8 { 1 }\n#[cfg(feature = "native")]\npub fn value() -> &\'static str { "native" }\n',
      );
      writeFileSync(
        join(root, "consumer/Cargo.toml"),
        '[package]\nname = "consumer"\nversion = "0.1.0"\nedition = "2021"\n[dependencies]\nshared = { path = "../shared" }\n[dev-dependencies]\nshared = { path = "../shared", features = ["native"] }\n',
      );
      writeFileSync(
        join(root, "consumer/src/lib.rs"),
        "pub fn consume() -> u8 { shared::value() }\n",
      );
      writeFileSync(
        join(root, "shell/Cargo.toml"),
        '[package]\nname = "shell"\nversion = "0.1.0"\nedition = "2021"\n[dependencies]\nconsumer = { path = "../consumer" }\nshared = { path = "../shared", features = ["native"] }\n',
      );
      writeFileSync(
        join(root, "shell/src/lib.rs"),
        "pub fn start() -> u8 { consumer::consume() }\n",
      );
      expect(run(["generate-lockfile", "--offline"]).status).toBe(0);
      const tree = (selected: string, edges: string) =>
        parseFeatureGraph(
          execFileSync(
            "cargo",
            [
              "tree",
              "--locked",
              "--offline",
              "-p",
              selected,
              "--edges",
              edges,
              "--prefix",
              "depth",
              "--format",
              "{p}|{f}",
            ],
            { cwd: root, encoding: "utf8" },
          ),
          root,
        );
      const common = tree("consumer", "normal,build");
      const native = tree("shell", "normal,build");
      const tests = tree("consumer", "normal,build,dev");
      expect(common.nodes.find((node) => node.name === "shared")!.features).toEqual([]);
      expect(native.nodes.find((node) => node.name === "shared")!.features).toContain("native");
      expect(tests.nodes.find((node) => node.name === "shared")!.features).toContain("native");
      expect(() => assertGraphMatches(common, tests)).toThrow("Dependency/feature graph changed");
      expect(
        run(["check", "--locked", "--offline", "-p", "shared", "--features", "native", "--lib"])
          .status,
      ).toBe(0);
      expect(run(["check", "--locked", "--offline", "-p", "consumer", "--lib"]).status).toBe(0);
      const production = run(["check", "--locked", "--offline", "-p", "shell", "--lib"]);
      expect(production.status).not.toBe(0);
      expect(production.stderr).toContain("mismatched types");
      const test = run(["test", "--locked", "--offline", "-p", "consumer", "--lib", "--no-run"]);
      expect(test.status).not.toBe(0);
      expect(test.stderr).toContain("mismatched types");
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  }, 30_000);
});
