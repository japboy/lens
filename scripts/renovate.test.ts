import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";
import { VERSION } from "release-please";
import { assertReleasePleaseVersion } from "./release/library-version.ts";

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
// Evaluate only this repository's trusted static JSON5 configuration, without globals.
const config = runInNewContext(`(${read("renovate.json5")})`, Object.create(null), {
  timeout: 1000,
}) as {
  enabledManagers: string[];
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

  it("groups both occurrences with manual review after the broad npm automerge rules", () => {
    const indices = config.packageRules.flatMap((rule, index) =>
      rule.matchPackageNames?.includes("release-please") ? [index] : [],
    );
    expect(indices).toHaveLength(1);
    const index = indices[0]!;
    const group = config.packageRules[index]!;
    expect(group.matchManagers).toEqual(["npm", "custom.regex"]);
    expect(group.groupName).toBe("release-please");
    expect(group.minimumGroupSize).toBe(2);
    expect(group.automerge).toBe(false);
    expect(group.matchUpdateTypes).toBeUndefined();
    expect(group.separateMajorMinor).toBeUndefined();
    expect(group.dependencyDashboardApproval).toBeUndefined();
    const autoRules = config.packageRules.flatMap((rule, position) =>
      rule.matchManagers?.includes("npm") && rule.automerge === true ? [position] : [],
    );
    expect(autoRules.length).toBeGreaterThan(0);
    expect(autoRules.every((position) => position < index)).toBe(true);
    expect(
      config.packageRules.some(
        (rule) =>
          rule.matchUpdateTypes?.includes("major") && rule.dependencyDashboardApproval === true,
      ),
    ).toBe(true);
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
