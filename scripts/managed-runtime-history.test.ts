import { describe, expect, it } from "vitest";
import {
  assertPreservedBootstrapIdentities,
  bootstrapIdentities,
} from "./managed-runtime-history.ts";
const old = { node: [["24.21.0", "node-old"]] as const, pnpm: [["11.22.0", "pnpm-old"]] as const };
describe("managed bootstrap identity retention", () => {
  it("permits either tool to update while retaining every approved prior tuple", () => {
    for (const node of [old.node, [...old.node, ["24.22.0", "node-next"] as const]]) {
      const next = { node: [...node], pnpm: [...old.pnpm, ["11.23.0", "pnpm-next"] as const] };
      expect(() =>
        assertPreservedBootstrapIdentities({ node: [...old.node], pnpm: [...old.pnpm] }, next),
      ).not.toThrow();
      const second = {
        node: [...next.node],
        pnpm: [...next.pnpm, ["11.24.0", "pnpm-later"] as const],
      };
      expect(() => assertPreservedBootstrapIdentities(next, second)).not.toThrow();
      expect(() =>
        assertPreservedBootstrapIdentities(next, { ...second, pnpm: second.pnpm.slice(1) }),
      ).toThrow("preserve");
    }
  });
  it("rejects lost old versions and changed digests under the same version", () => {
    for (const next of [
      { node: [["24.21.0", "wrong"]], pnpm: [...old.pnpm] },
      { node: [...old.node], pnpm: [["11.22.0", "wrong"]] },
      { node: [], pnpm: [...old.pnpm] },
    ]) {
      expect(() =>
        assertPreservedBootstrapIdentities(
          { node: [...old.node], pnpm: [...old.pnpm] },
          next as { node: [string, string][]; pnpm: [string, string][] },
        ),
      ).toThrow("preserve");
    }
  });
  it("migrates the base Rust SHA512 hex to the exact npm SRI identity", () => {
    const texts: Record<string, string> = {
      "mise.toml": '[tools]\nnode="24.21.0"',
      "mise.lock": `[[tools.node]]\nversion="24.21.0"\n[tools.node."platforms.macos-arm64"]\nchecksum="sha256:${"a".repeat(64)}"`,
      "apps/desktop/src-tauri/src/agent_runtime.rs": `const PNPM_VERSION: &str = "11.22.0";\nconst PNPM_ARCHIVE_SHA512: &str = "${"b".repeat(128)}";`,
    };
    expect(bootstrapIdentities((path) => texts[path] ?? null).pnpm).toEqual([
      ["11.22.0", `sha512:${"b".repeat(128)}`],
    ]);
  });
  it("migrates prior SRI policy to native lock while retaining the exact old tuple", () => {
    const config = '[tools]\nnode="24.21.0"\npnpm="12.6.0"';
    const lock = `[[tools.node]]\nversion="24.21.0"\n[tools.node."platforms.macos-arm64"]\nchecksum="sha256:${"a".repeat(64)}"\n[[tools.pnpm]]\nversion="12.6.0"\nbackend="aqua:pnpm/pnpm"\n[tools.pnpm."platforms.macos-arm64"]\nurl="https://github.com/pnpm/pnpm/releases/download/v12.6.0/pnpm-darwin-arm64.tar.gz"\nchecksum="sha256:${"c".repeat(64)}"`;
    const digest = "b".repeat(128);
    const texts: Record<string, string> = { "mise.toml": config, "mise.lock": lock };
    const legacy: Record<string, string> = {
      ...texts,
      "apps/desktop/src-tauri/agent-runtime/pnpm.toml": `version="11.22.0"\nintegrity="sha512-${Buffer.from(digest, "hex").toString("base64")}"`,
    };
    const historyPath = "apps/desktop/src-tauri/agent-runtime/pnpm-history.toml";
    const modern: Record<string, string> = {
      ...texts,
      [historyPath]: `[[previous]]\nversion="11.22.0"\narchive_digest="sha512:${digest}"`,
    };
    const before = bootstrapIdentities((path) => legacy[path] ?? null);
    const after = bootstrapIdentities((path) => modern[path] ?? null);
    expect(after.pnpm).toEqual([
      ["12.6.0", `sha256:${"c".repeat(64)}`],
      ["11.22.0", `sha512:${digest}`],
    ]);
    expect(() => assertPreservedBootstrapIdentities(before, after)).not.toThrow();
    for (const invalid of [
      { ...modern, [historyPath]: "previous=[]" },
      { ...modern, "mise.lock": lock.replace("sha256:cccc", "sha512:cccc") },
      { ...modern, [historyPath]: modern[historyPath].replace("sha512:", "sha256:") },
    ]) {
      expect(() =>
        assertPreservedBootstrapIdentities(
          before,
          bootstrapIdentities((path) => (invalid as Record<string, string>)[path] ?? null),
        ),
      ).toThrow(/pnpm|SHA256/u);
    }
  });
});
