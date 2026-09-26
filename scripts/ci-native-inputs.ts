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
]);

// Build scripts can compute paths or delegate reads. Pin their reviewed token surface
// and helper closure alongside inputs; any source change requires inventory review.
// This is intentionally a bounded inventory, not a claim to interpret arbitrary Rust.
export const NATIVE_BUILD_INPUTS = {
  "apps/desktop/src-tauri/build.rs": {
    digest: "9aa10aa24fc338c8617c361a161f3e6fdf91e50c63345a4ca5ff8be5af4cba66",
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
  "packages/adapter-platform-macos/build.rs": {
    digest: "a0cc159d4392a5e36c8c391e00ff495ae232f06843b383900764c021f5ff5060",
    inputs: [
      "packages/adapter-platform-macos/native/LensNative.m",
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
