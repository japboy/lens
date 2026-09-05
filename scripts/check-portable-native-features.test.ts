import { describe, expect, it } from "vitest";
import {
  assertProductionProjection,
  sharedProductionProjection,
  verificationManifest,
} from "../mise-tasks/check/rust/native-features.ts";

const tree = `0domain v0.1.0 (/fixture/packages/domain)|
1bridge v1.0.0|default
2shared v1.0.0|target
1derive v1.0.0 (proc-macro)|
2bridge v1.0.0|default
3shared v1.0.0|host

0port-platform v0.1.0 (/fixture/packages/port-platform)|
0use-case v0.1.0 (/fixture/packages/use-case)|
0desktop v0.1.0 (/fixture/apps/desktop)|
1sdk v1.0.0|native
`;
const dependency = (name: string) => ({ name, kind: null });
const packages = [
  {
    name: "domain",
    version: "0.1.0",
    source: null,
    dependencies: [dependency("bridge"), dependency("derive")],
  },
  { name: "port-platform", version: "0.1.0", source: null, dependencies: [] },
  { name: "use-case", version: "0.1.0", source: null, dependencies: [] },
  { name: "bridge", version: "1.0.0", source: "registry", dependencies: [dependency("shared")] },
  { name: "derive", version: "1.0.0", source: "registry", dependencies: [dependency("bridge")] },
];
const project = (output = tree) => sharedProductionProjection(output, "/fixture", packages);

describe("native-production shared feature projection", () => {
  it("keeps occurrence paths when equal-feature parents have different host/target children", () => {
    const projection = project();
    const shared = projection.nodes.filter((node) => node.package.name === "shared");
    expect(shared).toHaveLength(2);
    expect(shared.find((node) => node.context === "host")!.package.features).toEqual(["host"]);
    expect(shared.find((node) => node.context === "target")!.package.features).toEqual(["target"]);
    expect(projection.nodes.some((node) => node.package.name === "sdk")).toBe(false);
    expect(() => assertProductionProjection(projection, structuredClone(projection))).not.toThrow();
  });

  it.each([
    ["", /Incomplete shared/u],
    [tree.replace("2shared v1.0.0|target", "2shared v1.0.0|target (*)"), /non-deduplicated/u],
    [
      tree.replace("2shared v1.0.0|target", "4shared v1.0.0|target"),
      /Incomplete production tree depth/u,
    ],
    [tree.replace("0use-case v0.1.0 (/fixture/packages/use-case)|", ""), /Incomplete shared/u],
  ] as const)("rejects missing or approximate resolution evidence (%#)", (output, message) => {
    expect(() => project(output)).toThrow(message);
  });

  it("rejects ambiguous normal/build contexts", () => {
    const metadata = packages.map((entry) =>
      entry.name === "bridge"
        ? {
            ...entry,
            dependencies: [...entry.dependencies, { name: "shared", kind: "build" as const }],
          }
        : entry,
    );
    expect(() => sharedProductionProjection(tree, "/fixture", metadata)).toThrow(
      "Ambiguous production edge",
    );
  });

  it("detects native-only and host-only feature drift", () => {
    expect(() =>
      assertProductionProjection(project(), project(tree.replace("|target", "|native"))),
    ).toThrow("Native production feature projection differs");
    expect(() =>
      assertProductionProjection(project(), project(tree.replace("|host", "|different"))),
    ).toThrow("Native production feature projection differs");
  });

  it("generates an isolated nonpublishing resolver-2 workspace with exact features in distinct contexts", () => {
    const profiles =
      '[profile.release]\nopt-level = "s"\n[profile.release.build-override]\nopt-level = 1\n';
    const manifest = verificationManifest(
      "/fixture",
      project(),
      `[workspace]\nresolver = "2"\n${profiles}`,
    );
    expect(manifest).toContain(profiles);
    expect(manifest).toContain('[workspace]\nresolver = "2"');
    expect(manifest).toContain("publish = false");
    expect(manifest).toContain('path = "/fixture/packages/domain"');
    expect(manifest).toContain("[build-dependencies]");
    expect(manifest).toContain('version = "=1.0.0", default-features = false, features = ["host"]');
    expect(manifest).toContain(
      'version = "=1.0.0", default-features = false, features = ["target"]',
    );
  });

  it("rejects flattening multiple feature instances in one context", () => {
    const projection = project();
    projection.nodes.push({
      context: "target",
      package: { name: "shared", version: "1.0.0", source: "registry", features: ["extra"] },
    });
    expect(() => verificationManifest("/fixture", projection, "")).toThrow(
      "Multiple feature instances",
    );
  });
});
