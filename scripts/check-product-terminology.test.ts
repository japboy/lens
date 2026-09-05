import { describe, expect, it } from "vitest";
import {
  isProductTerminologyPath,
  productTerminologyViolations,
} from "./check-product-terminology.ts";

describe("product terminology policy", () => {
  it.each([
    ["README.md", true],
    ["apps/desktop/index.html", true],
    ["apps/desktop/src-tauri/tauri.conf.json", true],
    ["apps/desktop/src-tauri/tauri.macos.conf.json", true],
    ["apps/desktop/src/components/new-output.ts", true],
    ["apps/desktop/src/new-output.test.ts", true],
    ["apps/desktop/src-tauri/src/lib.rs", true],
    ["apps/desktop/src-tauri/native/macos/LensNative.m", true],
    ["packages/domain/src/projection.rs", true],
    ["apps/desktop/public/new-output.html", true],
    ["experiments/translation-continuity/results.json", false],
    ["apps/desktop/src-tauri/agent-runtime/codex/pnpm-lock.yaml", false],
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
    expect(
      productTerminologyViolations("apps/desktop/src/new-output.ts", `first line\r\n${term}`),
    ).toEqual([expect.stringContaining("apps/desktop/src/new-output.ts:2:")]);
  });

  it("rejects legacy filenames even when their content is empty", () => {
    expect(productTerminologyViolations("apps/desktop/src/translation.ts", "")).toEqual([
      "apps/desktop/src/translation.ts: product source path must use Interpretation",
    ]);
  });

  it("keeps shared package wording inside the product policy boundary", () => {
    expect(
      productTerminologyViolations("packages/domain/src/output.rs", "Translation"),
    ).toHaveLength(1);
  });

  it("keeps the exact language-conversion definition without exempting its file", () => {
    const definition = "- `Translation` is reserved for actual conversion between human languages.";
    expect(productTerminologyViolations("README.md", definition)).toEqual([]);
    expect(
      productTerminologyViolations("README.md", `${definition}\nTranslation is the default tab.`),
    ).toHaveLength(1);
    expect(productTerminologyViolations("apps/desktop/src/wording.ts", definition)).toHaveLength(1);
  });

  it("preserves technical terms, CSS positioning, and historical evidence", () => {
    expect(
      productTerminologyViolations(
        "apps/desktop/src/output.ts",
        "Interpretation LensRepresentation Transforming transform_current Agent Interpreter",
      ),
    ).toEqual([]);
    expect(
      productTerminologyViolations("apps/desktop/src/styles.css", "transform: translate(-50%, 0);"),
    ).toEqual([]);
    expect(
      productTerminologyViolations(
        "experiments/translation-continuity/trace.json",
        "living-translation-continuity-v1",
      ),
    ).toEqual([]);
  });
});
