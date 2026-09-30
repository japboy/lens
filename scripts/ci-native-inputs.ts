import { createHash } from "node:crypto";
import { posix } from "node:path";
import { rustTokens } from "./rust-source-boundaries.ts";

// These otherwise frontend-looking files are consumed by Rust code or build scripts.
export const NATIVE_CODE_INPUTS = new Set([
  "LICENSE",
  "NOTICE",
  "apps/desktop/tests/fixtures/workspace-contracts.json",
  "apps/desktop/tests/fixtures/acp-generated-image.json",
  "apps/desktop/src/html-output.ts",
  "apps/desktop/src/mcp-apps/sandbox-proxy.html",
  "apps/desktop/src/mcp-apps/sandbox-proxy.js",
  "apps/desktop/src/mcp-apps/rich-html-app.html",
]);

// Build scripts can compute paths or delegate reads. Pin their reviewed token surface
// and helper closure alongside inputs; any source change requires inventory review.
// This is intentionally a bounded inventory, not a claim to interpret arbitrary Rust.
export const NATIVE_BUILD_INPUTS = {
  "apps/desktop/src-tauri/build.rs": {
    digest: "489aa0cb408518084b1931dc0fb7254a79fb69476e0ed554dd9e199f18eda853",
    inputs: [
      "LICENSE",
      "NOTICE",
      "apps/desktop/src-tauri/tauri.conf.json",
      "apps/desktop/src-tauri/tauri.macos.conf.json",
    ],
  },
  "apps/desktop/src-tauri/node_policy.rs": {
    digest: "167a5e2557256749b2433d7832b7f1281cc5199fca9f556da22d68aed78b19f5",
    inputs: ["mise.toml", "mise.lock", "package.json", "apps/desktop/package.json"],
  },
  "apps/desktop/src-tauri/pnpm_policy.rs": {
    digest: "bcfa155bade8b2d422b2c615097033f2b9176607a6821c784655090dd77ebdc7",
    inputs: [
      "mise.toml",
      "mise.lock",
      "package.json",
      "apps/desktop/package.json",
      "apps/desktop/src-tauri/agent-runtime/pnpm-history.toml",
    ],
  },
  "apps/desktop/src-tauri/bootstrap_history.rs": {
    digest: "b02d640167e8302e7385a246a27651fc749682f9eb784fc9a0809e525630a502",
    inputs: ["apps/desktop/src-tauri/agent-runtime/node-history.toml"],
  },
  "packages/adapter-platform-macos/build.rs": {
    digest: "7369b195aa7d5debc2b35b6919db9f5364bd19b0ba5eb5f4cc9de3d49e83ed59",
    inputs: [
      "packages/adapter-platform-macos/native/LensNative.m",
      "packages/adapter-platform-macos/native/LensControlPalette.m",
      "packages/adapter-platform-macos/native/LensNative.h",
    ],
  },
} as const;

export function nativeBuildDigest(source: string): string {
  return createHash("sha256")
    .update(JSON.stringify(rustTokens(source).map(({ text }) => text)))
    .digest("hex");
}

export function rustIncludeInputs(owner: string, source: string): string[] {
  const tokens = rustTokens(source);
  const inputs: string[] = [];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if (
      token.literal ||
      tokens[index - 1]?.text === "." ||
      !["include", "include_str", "include_bytes"].includes(token.text)
    )
      continue;
    const literal = tokens[index + 3];
    const trailingComma = tokens[index + 4]?.text === ",";
    const close = tokens[index + (trailingComma ? 5 : 4)]?.text;
    if (
      tokens[index + 1]?.text !== "!" ||
      tokens[index + 2]?.text !== "(" ||
      !literal?.literal ||
      !/^"[^"\\\r\n]+"$/u.test(literal.text) ||
      close !== ")"
    )
      throw new Error(`Unreviewed Rust include expression: ${owner}`);
    const requested = literal.text.slice(1, -1);
    const input = posix.normalize(posix.join(posix.dirname(owner), requested));
    if (input.startsWith("../") || posix.isAbsolute(requested))
      throw new Error(`Native include escapes repository: ${owner}`);
    inputs.push(input);
  }
  return inputs;
}

export function nativeInputInventory(sources: ReadonlyMap<string, string>): Map<string, string[]> {
  const result = new Map<string, string[]>();
  for (const [owner, source] of sources) {
    const build = NATIVE_BUILD_INPUTS[owner as keyof typeof NATIVE_BUILD_INPUTS];
    if (posix.basename(owner) === "build.rs" && !build)
      throw new Error(`Unreviewed native build owner: ${owner}`);
    if (build && nativeBuildDigest(source) !== build.digest)
      throw new Error(`Native build input inventory requires review: ${owner}`);
    result.set(
      owner,
      [...new Set([...rustIncludeInputs(owner, source), ...(build?.inputs ?? [])])].sort(),
    );
  }
  for (const owner of Object.keys(NATIVE_BUILD_INPUTS)) {
    if (!sources.has(owner)) throw new Error(`Missing native build owner: ${owner}`);
  }
  return result;
}
