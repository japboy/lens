import { describe, expect, it } from "vitest";
import { graphDigest, parseFeatureGraph } from "../../mise-tasks/inspect/features.ts";
import { MEMBERS } from "../workspace-policy.ts";
import { admissionGraph } from "./graph.ts";

const members = MEMBERS.filter((member) => member.ecosystem === "cargo");
const graph = (version: string) =>
  parseFeatureGraph(
    members
      .map(
        (member) =>
          `0${member.name} v${version} (/fixture/${member.directory})|one\n1external v2.0.0|two`,
      )
      .join("\n"),
    "/fixture",
  );

describe("workspace release graph projection", () => {
  it("preserves raw observations while admitting synchronized workspace releases", () => {
    const before = graph("0.1.0");
    const after = graph("1.0.0");
    expect(graphDigest(before)).not.toBe(graphDigest(after));
    expect(admissionGraph(before, "0.1.0")).toEqual(admissionGraph(after, "1.0.0"));
  });

  it.each(members)("rejects an unsynchronized workspace member: $name", (member) => {
    const changed = graph("0.1.0");
    changed.nodes.find((node) => node.name === member.name)!.version = "0.2.0";
    expect(() => admissionGraph(changed, "0.1.0")).toThrow("disagrees");
  });

  it.each(["version", "features", "source", "edges", "roots"])(
    "retains external dependency %s changes",
    (field) => {
      const before = graph("0.1.0");
      const after = structuredClone(before);
      const external = after.nodes.find((node) => node.name === "external")!;
      if (field === "version") external.version = "3.0.0";
      if (field === "features") external.features.push("three");
      if (field === "source") external.source = "registry:proc-macro";
      if (field === "edges") after.edges.pop();
      if (field === "roots") after.roots.pop();
      expect(admissionGraph(after, "0.1.0")).not.toEqual(admissionGraph(before, "0.1.0"));
    },
  );

  it.each(members)(
    "preserves features and excludes same-name registry/different-path packages: $name",
    (member) => {
      const before = graph("0.1.0");
      const changed = structuredClone(before);
      changed.nodes.find((node) => node.name === member.name)!.features.push("three");
      expect(admissionGraph(changed, "0.1.0")).not.toEqual(admissionGraph(before, "0.1.0"));
      for (const source of ["registry", "registry:proc-macro", "path:another/package"]) {
        const external = structuredClone(before);
        const node = external.nodes.find((entry) => entry.name === member.name)!;
        node.source = source;
        const updated = structuredClone(external);
        updated.nodes.find((entry) => entry.name === member.name)!.version = "3.0.0";
        expect(admissionGraph(updated, "0.1.0")).not.toEqual(admissionGraph(external, "0.1.0"));
      }
    },
  );
});
