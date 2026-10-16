import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";
import { VERSION } from "release-please";
import { assertReleasePleaseVersion } from "./release/library-version.ts";
import { MEMBERS } from "./workspace-policy.ts";

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
// Evaluate only this repository's trusted static JSON5 configuration, without globals.
const config = runInNewContext(`(${read("renovate.json5")})`, Object.create(null), {
  timeout: 1000,
}) as {
  enabledManagers: string[];
  separateMinorPatch: boolean;
  customManagers: {
    depNameTemplate: string;
    datasourceTemplate: string;
    versioningTemplate: string;
    managerFilePatterns: string[];
    matchStrings: string[];
  }[];
  packageRules: {
    matchManagers?: string[];
    matchPackageNames?: string[];
    matchUpdateTypes?: string[];
    matchDepTypes?: string[];
    groupName?: string;
    minimumGroupSize?: number;
    automerge?: boolean;
    dependencyDashboardApproval?: boolean;
    separateMajorMinor?: boolean;
  }[];
};
const pin = JSON.parse(read("package.json")).devDependencies["release-please"] as string;
const schema = JSON.parse(read("release-please-config.json")).$schema as string;
const schemaFor = (version: string) =>
  `https://raw.githubusercontent.com/googleapis/release-please/v${version}/schemas/config.json`;

describe("Release Please dependency synchronization", () => {
  it("extracts the schema as the same npm dependency and limits the manager to its config", () => {
    expect(config.enabledManagers).toContain("custom.regex");
    const managers = config.customManagers.filter(
      (manager) => manager.depNameTemplate === "release-please",
    );
    expect(managers).toHaveLength(1);
    const manager = managers[0]!;
    expect(manager.datasourceTemplate).toBe("npm");
    expect(manager.versioningTemplate).toBe("npm");
    expect(manager.managerFilePatterns).toHaveLength(1);
    const pathPattern = new RegExp(manager.managerFilePatterns[0]!.slice(1, -1), "u");
    expect(pathPattern.test("release-please-config.json")).toBe(true);
    expect(pathPattern.test("nested/release-please-config.json")).toBe(false);
    expect(pathPattern.test("release-please-configXjson")).toBe(false);
    expect(manager.matchStrings).toHaveLength(1);
    const pattern = new RegExp(manager.matchStrings[0]!, "gu");
    const matches = [...read("release-please-config.json").matchAll(pattern)];
    expect(matches).toHaveLength(1);
    expect(matches[0]!.groups?.currentValue).toBe(pin);
    expect([...schema.replace("googleapis", "another-owner").matchAll(pattern)]).toHaveLength(0);
    expect([
      ...schema.replace("raw.githubusercontent.com", "rawXgithubusercontentXcom").matchAll(pattern),
    ]).toHaveLength(0);
    expect(
      schemaFor("18.1.2").match(new RegExp(manager.matchStrings[0]!, "u"))?.groups?.currentValue,
    ).toBe("18.1.2");
  });

  it("keeps the library and schema in one group across SemVer update levels", () => {
    const groups = config.packageRules.filter(
      (rule) => rule.matchPackageNames?.includes("release-please") && rule.groupName,
    );
    expect(groups).toHaveLength(1);
    const group = groups[0]!;
    expect(group.matchManagers).toEqual(["npm", "custom.regex"]);
    expect(group.groupName).toBe("release-please");
    expect(group.minimumGroupSize).toBe(2);
    expect(group.matchUpdateTypes).toBeUndefined();
    expect(group.separateMajorMinor).toBe(false);
    expect(config.separateMinorPatch).toBe(false);
    expect(group.automerge).toBeUndefined();
  });

  it("runs the installed library matching both exact repository pins", () => {
    expect(() => assertReleasePleaseVersion(pin, schema, VERSION)).not.toThrow();
  });

  it.each([
    ["range", "^18.1.2", schemaFor("18.1.2"), "18.1.2"],
    ["schema drift", "18.1.2", schemaFor("18.1.1"), "18.1.2"],
    ["installed drift", "18.1.2", schemaFor("18.1.2"), "18.1.1"],
    ["foreign schema", "18.1.2", "https://example.invalid/config.json", "18.1.2"],
  ])("rejects %s", (_name, dependency, schemaUrl, installed) => {
    expect(() => assertReleasePleaseVersion(dependency, schemaUrl, installed)).toThrow(
      /Release Please/u,
    );
  });
});

describe("Vite workspace synchronization", () => {
  it("uses one exact version across every adapter and consumer declaration", () => {
    const versions = MEMBERS.filter((member) => member.ecosystem === "pnpm").flatMap((member) => {
      const manifest = JSON.parse(read(`${member.directory}/package.json`)) as {
        dependencies?: Record<string, string>;
        devDependencies?: Record<string, string>;
      };
      return [manifest.dependencies?.vite, manifest.devDependencies?.vite].filter(
        (version): version is string => version !== undefined,
      );
    });
    expect(versions.length).toBeGreaterThan(0);
    expect(new Set(versions).size).toBe(1);
    expect(versions[0]).toMatch(/^\d+\.\d+\.\d+$/u);
  });

  it("overrides dependency-type groups without splitting Vite by update level", () => {
    const groups = config.packageRules.filter(
      (rule) => rule.matchPackageNames?.includes("vite") && rule.groupName,
    );
    expect(groups).toHaveLength(1);
    const group = groups[0]!;
    expect(group.matchManagers).toEqual(["npm"]);
    expect(group.groupName).toBe("Vite dependencies");
    expect(group.matchDepTypes).toBeUndefined();
    expect(group.matchUpdateTypes).toBeUndefined();
    expect(group.separateMajorMinor).toBe(false);
    expect(config.separateMinorPatch).toBe(false);
    const broadGroups = config.packageRules.filter(
      (rule) => rule.groupName && !rule.matchPackageNames && rule.matchManagers?.includes("npm"),
    );
    for (const broad of broadGroups)
      expect(config.packageRules.indexOf(group)).toBeGreaterThan(
        config.packageRules.indexOf(broad),
      );
  });

  it("keeps minor updates manual even when a consumer uses devDependencies", () => {
    const policy = config.packageRules.find(
      (rule) =>
        rule.matchPackageNames?.includes("vite") && rule.matchUpdateTypes?.includes("minor"),
    );
    expect(policy).toBeDefined();
    expect(policy!.matchDepTypes).toBeUndefined();
    expect(policy!.automerge).toBe(false);
    const automaticMinorRules = config.packageRules.filter(
      (rule) => rule.automerge && rule.matchUpdateTypes?.includes("minor"),
    );
    for (const automatic of automaticMinorRules)
      expect(config.packageRules.indexOf(policy!)).toBeGreaterThan(
        config.packageRules.indexOf(automatic),
      );
  });
});
