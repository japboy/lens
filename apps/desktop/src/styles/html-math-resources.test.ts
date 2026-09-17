import { describe, expect, it } from "vitest";
import { htmlMathPolicy, htmlMathResources } from "./html-math-resources";
import { HTML_PREVIEW_CSP } from "../html-output";

describe("external HTML math resources", () => {
  it("retains the admitted HTTP generation for development assets", () => {
    const resources = htmlMathResources("http://localhost:1420/overlay.html", "/_generations/abc/");
    expect(resources.stylesheet).toMatch(
      /^http:\/\/localhost:1420\/_generations\/abc\/assets\/html-math\/[a-f0-9]{64}\/katex\.css$/u,
    );
    expect(resources.fonts).toHaveLength(20);
    expect(
      resources.fonts.every((url) =>
        url.startsWith(resources.stylesheet.replace("katex.css", "fonts/")),
      ),
    ).toBe(true);
  });
  it.each([
    ["tauri://localhost/overlay.html", "lens-math://assets/"],
    ["http://tauri.localhost/overlay.html", "http://lens-math.assets/"],
  ])("uses only the public math protocol for packaged %s", (document, prefix) => {
    const resources = htmlMathResources(document, "/");
    expect(resources.stylesheet.startsWith(prefix + "assets/html-math/")).toBe(true);
    expect(resources.fonts.every((url) => url.startsWith(prefix))).toBe(true);
    expect(resources.stylesheet).not.toContain("data:");
  });
  it("allows exact trusted URLs while preserving every other policy directive", () => {
    const resources = htmlMathResources("tauri://localhost/overlay.html", "/");
    const policy = htmlMathPolicy(HTML_PREVIEW_CSP, resources);
    expect(policy).toContain(`style-src 'unsafe-inline' ${resources.stylesheet};`);
    expect(policy).toContain(`font-src data: ${resources.fonts.join(" ")};`);
    expect(policy.split("; ").filter((value) => !/^(style|font)-src /u.test(value))).toEqual(
      HTML_PREVIEW_CSP.split("; ").filter((value) => !/^(style|font)-src /u.test(value)),
    );
  });
});
