import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { MEMBERS } from "../workspace-policy.ts";
import {
  VERSION_FILES,
  compareVersions,
  manifestVersionState,
  tagVersion,
  versionState,
} from "./version.ts";

const cargo = MEMBERS.filter((member) => member.ecosystem === "cargo");
const files = (version = "0.1.0", bootstrapped = false) => {
  const value = Object.fromEntries(VERSION_FILES.map((path) => [path, readFileSync(path, "utf8")]));
  const current = JSON.parse(value["apps/desktop/src-tauri/tauri.conf.json"]!).version as string;
  for (const path of VERSION_FILES.filter(
    (path) => path.endsWith(".json") || path === "Cargo.toml",
  ))
    value[path] = value[path]!.replace(
      `"version": "${current}"`,
      `"version": "${version}"`,
    ).replace(`version = "${current}"`, `version = "${version}"`);
  for (const member of cargo)
    value["Cargo.lock"] = value["Cargo.lock"]!.replace(
      `name = "${member.name}"\nversion = "${current}"`,
      `name = "${member.name}"\nversion = "${version}"`,
    );
  value[".release-please-manifest.json"] = JSON.stringify(bootstrapped ? { ".": version } : {});
  return value;
};

describe("workspace release authority", () => {
  it("admits manifest proposals but requires a synchronized lock before release admission", () => {
    const proposal = files("0.3.0", true);
    proposal["Cargo.lock"] = files("0.2.0", true)["Cargo.lock"]!;
    expect(manifestVersionState(proposal)).toEqual({ version: "0.3.0", bootstrapped: true });
    expect(() => versionState(proposal)).toThrow("disagree");
  });
  it("admits initial unshipped state and synchronized patch, minor and major releases", () => {
    expect(versionState(files())).toEqual({ version: "0.1.0", bootstrapped: false });
    for (const version of ["0.1.1", "0.2.0", "1.0.0"])
      expect(versionState(files(version, true))).toEqual({ version, bootstrapped: true });
    expect(() => versionState(files("0.2.0"))).toThrow("Only initial");
  });
  it.each(VERSION_FILES)("rejects an inconsistent mirror or inheritance: %s", (file) => {
    const changed = files();
    changed[file] = file.endsWith("/Cargo.toml")
      ? changed[file]!.replace("version.workspace = true", 'version = "0.1.0"')
      : file.endsWith("manifest.json")
        ? '{".":"0.2.0"}'
        : changed[file]!.replace("0.1.0", "0.2.0");
    expect(() => versionState(changed)).toThrow(/.+/u);
  });
  it.each(cargo.map((member) => member.name))("rejects a stale lock version for %s", (name) => {
    const changed = files("1.0.0", true);
    changed["Cargo.lock"] = changed["Cargo.lock"]!.replace(
      `name = "${name}"\nversion = "1.0.0"`,
      `name = "${name}"\nversion = "0.1.0"`,
    );
    expect(() => versionState(changed)).toThrow("disagree");
  });
  it.each(["missing", "duplicate", "source", "unexpected"])(
    "rejects %s local lock identity",
    (kind) => {
      const changed = files();
      const entry = '[[package]]\nname = "domain"\nversion = "0.1.0"\n';
      if (kind === "missing")
        changed["Cargo.lock"] = changed["Cargo.lock"]!.replace('name = "domain"', 'name = "other"');
      if (kind === "duplicate") changed["Cargo.lock"] += entry;
      if (kind === "source")
        changed["Cargo.lock"] = changed["Cargo.lock"]!.replace(
          'name = "domain"',
          'name = "domain"\nsource = "registry+fixture"',
        );
      if (kind === "unexpected") changed["Cargo.lock"] += entry.replace("domain", "other");
      expect(() => versionState(changed)).toThrow(/Cargo.lock/u);
    },
  );
  it("allows independent registry versions even when they share a workspace name", () => {
    const changed = files();
    changed["Cargo.lock"] +=
      '\n[[package]]\nname = "domain"\nversion = "9.9.9"\nsource = "registry+fixture"\n';
    expect(versionState(changed).version).toBe("0.1.0");
  });
  it.each(cargo)("rejects incorrect identity or missing inheritance for $name", (member) => {
    for (const replacement of ['name = "other"', "version.workspace = false"]) {
      const changed = files();
      const path = join(member.directory, "Cargo.toml");
      changed[path] = changed[path]!.replace(
        replacement.startsWith("name") ? `name = "${member.name}"` : "version.workspace = true",
        replacement,
      );
      expect(() => versionState(changed)).toThrow("identity and inherited");
    }
  });
  it.each(["0.1.0", "v01.0.0", "v1.0", "v1.0.0-rc.1", "v1.0.0+build", "v1.0.0\n", "v1.0.0/evil"])(
    "rejects malformed tag %s",
    (tag) => {
      expect(() => tagVersion(tag)).toThrow(/.+/u);
    },
  );
  it("accepts equivalent TOML keys, whitespace, comments and inheritance tables", () => {
    const changed = files();
    changed["Cargo.toml"] = changed["Cargo.toml"]!.replace(
      'version = "0.1.0"',
      "\"version\"='0.1.0' # unchanged",
    );
    for (const member of cargo) {
      const path = join(member.directory, "Cargo.toml");
      changed[path] = changed[path]!.replace(
        `name = "${member.name}"`,
        `"name"='${member.name}' # identity`,
      ).replace("version.workspace = true", "version={workspace=true} # inherited");
    }
    changed["Cargo.lock"] = changed["Cargo.lock"]!.replaceAll(
      "[[package]]",
      "[[ package ]] # package entry",
    ).replaceAll('version = "0.1.0"', '"version"="0.1.0" # unchanged');
    expect(versionState(changed)).toEqual({ version: "0.1.0", bootstrapped: false });
  });
  it.each([
    "Cargo.toml",
    "Cargo.lock",
    ...cargo.map((member) => join(member.directory, "Cargo.toml")),
  ])("rejects duplicate quoted keys and malformed TOML with the input path: %s", (path) => {
    for (const malformed of [false, true]) {
      const changed = files();
      changed[path] = malformed
        ? `${changed[path]}\n[unterminated`
        : changed[path]!.replace(
            /^(version = "[^"\n]+"|version.workspace = true)$/mu,
            (line) => `${line}\n"version" = "0.1.0"`,
          );
      expect(() => versionState(changed)).toThrow(`${path}: invalid TOML`);
    }
  });
  it.each(["name = 1", "version = 1", "source = false", 'source = ""', "checksum = 1"])(
    "rejects wrongly typed or empty lock identity fields: %s",
    (field) => {
      const changed = files();
      const key = field.split(" = ")[0]!;
      changed["Cargo.lock"] +=
        `\n[[package]]\n${key === "name" ? "" : 'name = "external"\n'}${key === "version" ? "" : 'version = "1.0.0"\n'}${key === "source" ? "" : 'source = "registry+fixture"\n'}${field}\n`;
      expect(() => versionState(changed)).toThrow(`Cargo.lock: expected a nonempty package ${key}`);
    },
  );
  it.each([
    'version = "0.1.0"',
    'version = { workspace = "true" }',
    "version = { workspace = true, extra = true }",
  ])("requires only a true workspace inheritance field: %s", (version) => {
    const changed = files();
    const path = join(cargo[0]!.directory, "Cargo.toml");
    changed[path] = changed[path]!.replace("version.workspace = true", version);
    expect(() => versionState(changed)).toThrow("identity and inherited");
  });
  it.each(["package = 1", "package = [1]", '[package]\nname = "domain"', "package = [1979-05-27]"])(
    "rejects malformed lock package tables: %s",
    (lock) => {
      const changed = files();
      changed["Cargo.lock"] = lock;
      expect(() => versionState(changed)).toThrow(/Cargo.lock/u);
    },
  );
  it("compares unbounded stable numeric components without lexical ordering", () => {
    expect(tagVersion("v1.2.3")).toBe("1.2.3");
    expect(compareVersions("0.9.0", "0.10.0")).toBe(-1);
    expect(compareVersions("1.0.0", "0.100.0")).toBe(1);
    expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
  });
});
