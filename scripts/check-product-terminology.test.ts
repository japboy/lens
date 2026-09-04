import { describe, expect, it } from "vitest";
import {
  isProductTerminologyPath,
  productTerminologyViolations,
} from "./check-product-terminology.ts";

describe("product terminology policy", () => {
  it.each([
    ["README.md", true],
    ["index.html", true],
    ["src-tauri/tauri.conf.json", true],
    ["src/components/new-output.ts", true],
    ["src/new-output.test.ts", true],
    ["src-tauri/src/lib.rs", true],
    ["src-tauri/native/macos/LensNative.m", true],
    ["public/new-output.html", true],
    ["experiments/translation-continuity/results.json", false],
    ["src-tauri/agent-runtime/codex/pnpm-lock.yaml", false],
    ["scripts/check-product-terminology.test.ts", false],
  ] as const)("classifies %s as product source: %s", (path, expected) => {
    expect(isProductTerminologyPath(path)).toBe(expected);
  });

  it.each([
    "Translation",
    "translations",
    "TRANSLATION",
    "translation-panel",
    "TranslationScrollPosition",
  ])("rejects the legacy product term %s", (term) => {
    expect(productTerminologyViolations("src/new-output.ts", `first line\r\n${term}`)).toEqual([
      expect.stringContaining("src/new-output.ts:2:"),
    ]);
  });

  it("rejects legacy filenames even when their content is empty", () => {
    expect(productTerminologyViolations("src/translation.ts", "")).toEqual([
      "src/translation.ts: product source path must use Interpretation",
    ]);
  });

  it("keeps the exact language-conversion definition without exempting its file", () => {
    const definition = "- `Translation` is reserved for actual conversion between human languages.";
    expect(productTerminologyViolations("README.md", definition)).toEqual([]);
    expect(
      productTerminologyViolations("README.md", `${definition}\nTranslation is the default tab.`),
    ).toHaveLength(1);
    expect(productTerminologyViolations("src/wording.ts", definition)).toHaveLength(1);
  });

  it("preserves technical terms, CSS positioning, and historical evidence", () => {
    expect(
      productTerminologyViolations(
        "src/output.ts",
        "Interpretation LensRepresentation Transforming transform_current Agent Interpreter",
      ),
    ).toEqual([]);
    expect(
      productTerminologyViolations("src/styles.css", "transform: translate(-50%, 0);"),
    ).toEqual([]);
    expect(
      productTerminologyViolations(
        "experiments/translation-continuity/trace.json",
        "living-translation-continuity-v1",
      ),
    ).toEqual([]);
  });
});
