import { describe, expect, it } from "vitest";
import { ignoredResourceReferences } from "./publication.ts";

const localDirectory = ["docs", "/"].join("");
const localFile = ["ARCHITECTURE", ".md"].join("");

describe("publication references", () => {
  it.each([
    "https://www.apple.com/legal/sla/" + localDirectory + "xcode.pdf",
    `<https://example.com/${localFile}>`,
    `[license](https://example.com/${localDirectory}license)`,
  ])("allows external web resources: %s", (line) => {
    expect(ignoredResourceReferences(line)).toEqual([]);
  });

  it.each([
    localDirectory + "design.md",
    `[design](${localDirectory}design.md)`,
    `https://example.com/license ${localDirectory}design.md`,
    `[${localDirectory}design.md](https://example.com/license)`,
    `https://example.com/license"${localDirectory}design.md`,
    `file:///checkout/${localDirectory}design.md`,
  ])("still rejects local resources: %s", (line) => {
    expect(ignoredResourceReferences(line)).toEqual([localDirectory]);
  });

  it("retains other local-only resource checks beside a web URL", () => {
    expect(ignoredResourceReferences(`https://example.com/license ${localFile}`)).toEqual([
      localFile,
    ]);
  });
});
