import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";
import { MODE_CONFIG_SENTINEL } from "./types";

// The sentinel is a cross-language contract: the settings UI keys a saved mode choice by
// it and the Rust validators look it up under the same id. Nothing links the two
// declarations, so pin the TypeScript one against the Rust constant the contract fixture
// records.
it("keys a saved mode choice by the same sentinel the Rust validators use", () => {
  const fixture = JSON.parse(
    readFileSync(
      fileURLToPath(new URL("../tests/fixtures/workspace-contracts.json", import.meta.url)),
      "utf8",
    ),
  );
  expect(fixture.mode_config_sentinel).toBe(MODE_CONFIG_SENTINEL);
});
