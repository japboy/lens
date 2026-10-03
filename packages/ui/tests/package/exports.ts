import { createRequire } from "node:module";
import { expect, it } from "vitest";

const resolve = createRequire(import.meta.url).resolve;

it.each([
  "ui",
  "ui/ssr",
  "ui/contracts",
  "ui/contracts/resource-state",
  "ui/contracts/response-history",
  "ui/components/views/lens-overlay-view",
  "ui/components/controls/lens-select",
  "ui/entries/overlay-conversation",
  "ui/entries/settings-prompts",
  "ui/resources/document-host",
  "ui/styles/document.css",
  "ui/styles/icon-fonts.css",
  "ui/styles/math-fonts.css",
  "ui/test-fixtures/media-parity",
])("resolves the application-facing package entry %s", (entry) => {
  expect(resolve(entry)).toMatch(/\.(?:ts|css)$/u);
});

it.each([
  "ui/components/views/lens-overlay-view.test",
  "ui/rendering/streaming-markdown.test",
  "ui/resources/conversation-render-cache",
  "ui/rendering/streaming-markdown",
  "ui/test-setup",
  "ui/tests/fixtures/media-parity",
  "ui/src/index.ts",
])("rejects the private source or test entry %s", (entry) => {
  expect(() => resolve(entry)).toThrowError(
    expect.objectContaining({ code: "ERR_PACKAGE_PATH_NOT_EXPORTED" }),
  );
});
