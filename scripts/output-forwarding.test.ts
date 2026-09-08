import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

describe("managed adapter HTML compatibility contracts", () => {
  it.each(["codex", "claude"])("checks %s publisher-only forwarding", (adapter) => {
    const suite = fileURLToPath(
      new URL(
        `../apps/desktop/src-tauri/agent-runtime/${adapter}/lens-html-forwarding.test.mjs`,
        import.meta.url,
      ),
    );
    expect(execFileSync(process.execPath, [suite], { encoding: "utf8" })).toContain(
      "checks passed",
    );
  });
});
