import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { VERSION_FILES, compareVersions, tagVersion, versionState } from "./version.ts";
import { admissionGraph } from "./graph.ts";
import { graphDigest, parseFeatureGraph } from "../../mise-tasks/inspect/features.ts";

const files = () => {
  const value = Object.fromEntries(VERSION_FILES.map((path) => [path, readFileSync(path, "utf8")]));
  const current = JSON.parse(value[VERSION_FILES[0]]!).version as string;
  for (const path of VERSION_FILES.slice(0, 4))
    value[path] = value[path]!.replace(`"version": "${current}"`, '"version": "0.1.0"').replace(
      `version = "${current}"`,
      'version = "0.1.0"',
    );
  value["Cargo.lock"] = value["Cargo.lock"]!.replace(
    `name = "desktop"\nversion = "${current}"`,
    'name = "desktop"\nversion = "0.1.0"',
  );
  value[".release-please-manifest.json"] = "{}";
  return value;
};
describe("application release authority", () => {
  it("admits initial unshipped state and synchronized release versions", () => {
    expect(versionState(files())).toEqual({ version: "0.1.0", bootstrapped: false });
    for (const version of ["0.1.1", "0.2.0", "1.0.0"]) {
      const changed = files();
      for (const path of VERSION_FILES.slice(0, 4))
        changed[path] = changed[path]!.replace(
          '"version": "0.1.0"',
          `"version": "${version}"`,
        ).replace('version = "0.1.0"', `version = "${version}"`);
      changed["Cargo.lock"] = changed["Cargo.lock"]!.replace(
        'name = "desktop"\nversion = "0.1.0"',
        `name = "desktop"\nversion = "${version}"`,
      );
      changed[".release-please-manifest.json"] = JSON.stringify({ ".": version });
      expect(versionState(changed)).toEqual({ version, bootstrapped: true });
    }
  });
  it.each(VERSION_FILES)("rejects an inconsistent mirror: %s", (file) => {
    const changed = files();
    changed[file] =
      file === "Cargo.lock"
        ? changed[file]!.replace(
            'name = "desktop"\nversion = "0.1.0"',
            'name = "desktop"\nversion = "0.2.0"',
          )
        : file.endsWith("manifest.json")
          ? '{".":"0.2.0"}'
          : changed[file]!.replace("0.1.0", "0.2.0");
    expect(() => versionState(changed)).toThrow(/.+/u);
  });
  it.each(["0.1.0", "v01.0.0", "v1.0", "v1.0.0-rc.1", "v1.0.0+build", "v1.0.0\n", "v1.0.0/evil"])(
    "rejects malformed tag %s",
    (tag) => {
      expect(() => tagVersion(tag)).toThrow(/.+/u);
    },
  );
  it("compares unbounded stable numeric components without lexical ordering", () => {
    expect(tagVersion("v1.2.3")).toBe("1.2.3");
    expect(compareVersions("0.9.0", "0.10.0")).toBe(-1);
    expect(compareVersions("1.0.0", "0.100.0")).toBe(1);
    expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
  });
});

describe("application-only graph projection", () => {
  const graph = (version: string) =>
    parseFeatureGraph(
      `0desktop v${version} (/fixture/apps/desktop/src-tauri)|\n1shared v0.1.0 (/fixture/packages/shared)|one\n1desktop v1.0.0|two\n`,
      "/fixture",
    );
  it("preserves observations but admits only a synchronized application version change", () => {
    const before = graph("0.1.0");
    const after = graph("1.0.0");
    expect(graphDigest(before)).not.toBe(graphDigest(after));
    expect(admissionGraph(before, "0.1.0")).toEqual(admissionGraph(after, "1.0.0"));
    expect(() => admissionGraph(after, "0.1.0")).toThrow("disagrees");
  });
  it.each(["version", "features", "source", "edges", "roots"])(
    "retains dependency %s changes",
    (field) => {
      const before = graph("0.1.0");
      const after = structuredClone(before);
      const shared = after.nodes.find((node) => node.name === "shared")!;
      if (field === "version") shared.version = "0.2.0";
      if (field === "features") shared.features.push("two");
      if (field === "source") shared.source = "registry";
      if (field === "edges") after.edges.pop();
      if (field === "roots") after.roots.push(1);
      expect(admissionGraph(after, "0.1.0")).not.toEqual(admissionGraph(before, "0.1.0"));
    },
  );
  it("does not normalize same-name registry or different-path packages", () => {
    const before = graph("0.1.0");
    for (const source of ["registry", "path:another/desktop"]) {
      const changed = structuredClone(before);
      const node = changed.nodes.find(
        (entry) => entry.name === "desktop" && entry.source === "registry",
      )!;
      node.source = source;
      node.version = "2.0.0";
      expect(admissionGraph(changed, "0.1.0")).not.toEqual(admissionGraph(before, "0.1.0"));
    }
  });
});
