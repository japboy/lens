import { dirname, resolve } from "node:path";
import { RUST_SAFETY_BOUNDARIES, MIRI_EXCLUSIONS } from "./rust-safety-boundaries.ts";

type Token = { text: string; literal: boolean; offset: number };

// A conservative lexical policy, not a Rust parser or macro-expansion proof. Rustc
// checks syntax/types; the finite rules below reject unreviewed escape mechanisms.
export function rustTokens(source: string): Token[] {
  const tokens: Token[] = [];
  let offset = 0;
  while (offset < source.length) {
    const rest = source.slice(offset);
    if (/^\s/u.test(rest)) {
      offset += 1;
      continue;
    }
    if (rest.startsWith("//")) {
      const end = source.indexOf("\n", offset);
      offset = end === -1 ? source.length : end;
      continue;
    }
    if (rest.startsWith("/*")) {
      let depth = 1;
      offset += 2;
      while (depth && offset < source.length) {
        if (source.startsWith("/*", offset)) {
          depth += 1;
          offset += 2;
        } else if (source.startsWith("*/", offset)) {
          depth -= 1;
          offset += 2;
        } else offset += 1;
      }
      if (depth) throw new Error("Unterminated Rust block comment");
      continue;
    }
    const raw = /^(?:br|cr|r)(#{0,255})"/u.exec(rest);
    if (raw) {
      const closing = `"${raw[1]}`;
      const end = source.indexOf(closing, offset + raw[0].length);
      if (end === -1) throw new Error("Unterminated Rust raw string");
      const length = end + closing.length - offset;
      tokens.push({ text: rest.slice(0, length), literal: true, offset });
      offset += length;
      continue;
    }
    const quoted = /^(?:b|c)?"/u.exec(rest);
    if (quoted) {
      let end = offset + quoted[0].length;
      while (end < source.length && source[end] !== '"') {
        end += source[end] === "\\" ? 2 : 1;
      }
      if (end >= source.length) throw new Error("Unterminated Rust string");
      tokens.push({ text: source.slice(offset, end + 1), literal: true, offset });
      offset = end + 1;
      continue;
    }
    const character =
      /^(?:b)?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|[^\r\n])|[^'\\\r\n])'/u.exec(rest);
    if (character) {
      tokens.push({ text: character[0], literal: true, offset });
      offset += character[0].length;
      continue;
    }
    const identifier = /^(?:r#)?[A-Za-z_][A-Za-z_0-9]*/u.exec(rest);
    if (identifier) {
      tokens.push({ text: identifier[0].replace(/^r#/u, ""), literal: false, offset });
      offset += identifier[0].length;
      continue;
    }
    if (rest.codePointAt(0)! > 127)
      throw new Error("Non-ASCII Rust identifier requires boundary review");
    const punctuation = rest.startsWith("::") ? "::" : rest[0]!;
    tokens.push({ text: punctuation, literal: false, offset });
    offset += punctuation.length;
  }
  return tokens;
}

export const RUST_GROUP_CLOSING = new Map([
  ["(", ")"],
  ["[", "]"],
  ["{", "}"],
]);

export function groupEnd(tokens: readonly Token[], start: number): number {
  const expected = RUST_GROUP_CLOSING.get(tokens[start]?.text ?? "");
  if (!expected) throw new Error("Expected a Rust token group");
  for (let index = start + 1; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if (token.literal) continue;
    if (token.text === expected) return index;
    if (RUST_GROUP_CLOSING.has(token.text)) index = groupEnd(tokens, index);
    else if ([")", "]", "}"].includes(token.text)) throw new Error("Mismatched Rust token group");
  }
  throw new Error("Incomplete Rust token group");
}

const PORTABLE_FORBIDDEN = new Set([
  "unsafe",
  "extern",
  "macro",
  "macro_rules",
  "cfg_attr",
  "cfg_if",
  "include",
  "include_str",
  "include_bytes",
  "env",
  "option_env",
  "target_os",
  "target_arch",
  "target_family",
  "tauri",
  "adapter_platform_macos",
  "objc",
  "objc2",
  "libc",
  "winapi",
  "fs",
  "io",
  "process",
  "thread",
  "net",
  "os",
  "SystemTime",
  "Instant",
  "new_v4",
  "now_v7",
  "exists",
  "try_exists",
  "is_file",
  "is_dir",
  "is_symlink",
  "read_dir",
  "read_link",
  "symlink_metadata",
]);
const PORTABLE_MACROS = new Set([
  "assert",
  "assert_eq",
  "assert_ne",
  "concat",
  "format",
  "json",
  "matches",
  "panic",
  "pin",
  "unreachable",
  "vec",
  "write",
]);
const PORTABLE_ATTRIBUTES = new Set([
  "cfg",
  "default",
  "derive",
  "doc",
  "error",
  "forbid",
  "serde",
  "source",
  "test",
  "tokio::test",
]);

// Lint attributes are checked lexically in every authored Rust target, including
// tests and build scripts. Rustc remains responsible for expanded Rust semantics.
export function unsafeManifestViolations(manifest: unknown, workspaceRoot = false): string[] {
  const document = manifest as {
    workspace?: { lints?: { rust?: Record<string, unknown> } };
    lints?: Record<string, unknown>;
  };
  if (workspaceRoot) {
    return ["unsafe_code", "unsafe_op_in_unsafe_fn"]
      .filter((lint) => document.workspace?.lints?.rust?.[lint] !== "deny")
      .map((lint) => `Workspace must deny ${lint}`);
  }
  return document.lints?.workspace === true && Object.keys(document.lints).length === 1
    ? []
    : ["Cargo targets must inherit workspace Rust safety lints without overrides"];
}

export function miriExclusionViolations(source: string, owner: string): string[] {
  const tokens = rustTokens(source);
  for (let index = 0; index < tokens.length; index += 1) {
    if (
      !tokens[index]?.literal &&
      tokens[index]?.text === "cfg" &&
      tokens[index + 1]?.text === "!"
    ) {
      const end = groupEnd(tokens, index + 2);
      if (tokens.slice(index, end).some((token) => !token.literal && token.text === "miri"))
        return ["Miri-specific execution branches must not omit production processing"];
    }
  }
  const selectedOwner = Object.values(RUST_SAFETY_BOUNDARIES).some(
    (boundary) =>
      boundary.miriGroups.length > 0 && (boundary.owners as readonly string[]).includes(owner),
  );
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index]?.text !== "#") continue;
    const start = index + (tokens[index + 1]?.text === "!" ? 2 : 1);
    const end = groupEnd(tokens, start);
    const attribute = tokens.slice(index, end);
    if (
      !attribute.some(
        (token) =>
          !token.literal && (token.text === "miri" || (selectedOwner && token.text === "ignore")),
      )
    )
      continue;
    let declaration = end + 1;
    while (tokens[declaration]?.text === "#" && tokens[declaration + 1]?.text === "[") {
      declaration = groupEnd(tokens, declaration + 1) + 1;
    }
    const test = tokens[declaration]?.text === "fn" ? tokens[declaration + 1]?.text : undefined;
    if (
      !MIRI_EXCLUSIONS.some(
        (entry) =>
          entry.owner === owner && test && entry.test.endsWith(`::${test}`) && entry.reason.trim(),
      )
    )
      return [
        "Miri exclusions or ignored boundary tests require an exact declared test and justification",
      ];
  }
  return [];
}

export function unsafeExceptionViolations(source: string, owner?: string): string[] {
  const tokens = rustTokens(source);
  const violations: string[] = [];
  const allowedTargets = new Set<number>();
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index]?.text !== "#") continue;
    const inner = tokens[index + 1]?.text === "!";
    const start = index + (inner ? 2 : 1);
    if (tokens[start]?.text !== "[") continue;
    let end = start + 1;
    let depth = 1;
    while (end < tokens.length && depth) {
      if (tokens[end]?.text === "[") depth += 1;
      if (tokens[end]?.text === "]") depth -= 1;
      end += 1;
    }
    const attribute = tokens.slice(start + 1, end - 1);
    const texts = attribute.map((token) => token.text);
    if (!texts.some((text) => text === "unsafe_code" || text === "unsafe_op_in_unsafe_fn"))
      continue;
    if (!texts.some((text) => ["allow", "warn", "expect"].includes(text))) continue;
    // The reason value follows '='; accept Rust string literals, but not empty reasons.
    const value = attribute[6];
    const documented =
      !inner &&
      texts[0] === "allow" &&
      texts[1] === "(" &&
      texts[2] === "unsafe_code" &&
      texts[3] === "," &&
      texts[4] === "reason" &&
      texts[5] === "=" &&
      value?.literal &&
      value.text
        .replace(/^(?:r#*|b)?"/u, "")
        .replace(/"#*$/u, "")
        .trim().length > 0 &&
      texts.slice(7).join("").replace(",", "") === ")";
    if (!documented) {
      violations.push('Unsafe exceptions require one outer allow(unsafe_code, reason = "...")');
      continue;
    }
    let reason: string;
    try {
      reason = JSON.parse(value!.text) as string;
    } catch {
      violations.push("Boundary reasons require an explicit ordinary quoted string");
      continue;
    }
    const category = /^\[boundary:([a-z-]+)\] (.+)$/su.exec(reason);
    const boundary = category
      ? RUST_SAFETY_BOUNDARIES[category[1] as keyof typeof RUST_SAFETY_BOUNDARIES]
      : undefined;
    if (!boundary || !category?.[2]?.trim())
      violations.push("Unsafe exception requires a declared boundary category and detailed reason");
    else if (owner && !(boundary.owners as readonly string[]).includes(owner))
      violations.push(`Unsafe boundary ${category![1]} cannot be used by ${owner}`);
    let target = end;
    // Other outer attributes can precede the actual unsafe operation.
    while (tokens[target]?.text === "#" && tokens[target + 1]?.text === "[") {
      target += 2;
      let nesting = 1;
      while (target < tokens.length && nesting) {
        if (tokens[target]?.text === "[") nesting += 1;
        if (tokens[target]?.text === "]") nesting -= 1;
        target += 1;
      }
    }
    if (tokens[target]?.text === "pub") {
      target += 1;
      if (tokens[target]?.text === "(") {
        while (target < tokens.length && tokens[target]?.text !== ")") target += 1;
        target += 1;
      }
    }
    if (tokens[target]?.text !== "unsafe")
      violations.push("Unsafe exceptions must attach directly to an unsafe block or declaration");
    else allowedTargets.add(target);
  }
  for (let index = 0; index < tokens.length; index += 1) {
    if (
      tokens[index]?.text === "unsafe" &&
      tokens[index + 1]?.text === "{" &&
      !allowedTargets.has(index)
    )
      violations.push(
        `Unsafe block at ${tokens[index]!.offset} requires its own documented exception`,
      );
  }
  return violations;
}

export function portableSourceViolations(source: string): string[] {
  const tokens = rustTokens(source);
  const violations: string[] = [];
  for (const [index, token] of tokens.entries()) {
    if (token.literal) continue;
    const at = (distance: number) => tokens[index + distance]?.text;
    if (PORTABLE_FORBIDDEN.has(token.text))
      violations.push(`Forbidden portable token ${token.text} at ${token.offset}`);
    if (token.text === "windows" && at(1) === "::")
      violations.push(`Native API path at ${token.offset}`);
    if (token.text === "cfg" && [at(1), at(2), at(3)].join(" ") !== "( test )") {
      violations.push(`Only cfg(test) is admitted at ${token.offset}`);
    }
    if (
      /^[A-Za-z_]/u.test(token.text) &&
      at(1) === "!" &&
      ["(", "[", "{"].includes(at(2) ?? "") &&
      !PORTABLE_MACROS.has(token.text)
    ) {
      violations.push(`Unreviewed portable macro ${token.text} at ${token.offset}`);
    }
    if (token.text === "#") {
      const start = index + (at(1) === "!" ? 3 : 2);
      let end = start;
      while (tokens[end + 1]?.text === "::") end += 2;
      const name = tokens
        .slice(start, end + 1)
        .map((entry) => entry.text)
        .join("");
      if (!PORTABLE_ATTRIBUTES.has(name))
        violations.push(`Unreviewed portable attribute ${name} at ${token.offset}`);
    }
  }
  return violations;
}

// Literal resources must remain within the logical application/package owner.
// Source inclusion is disallowed even within an owner. Path attributes in the shell
// must be literal and owner-local; portable code rejects path attributes entirely.
export function sourceInclusionViolations(
  root: string,
  path: string,
  source: string,
  owner: string,
): string[] {
  const tokens = rustTokens(source);
  const violations: string[] = [];
  for (const [index, token] of tokens.entries()) {
    if (token.literal) continue;
    if (token.text === "#") {
      let depth = 0;
      for (let cursor = index + 1; cursor < tokens.length; cursor += 1) {
        const attribute = tokens[cursor]!;
        if (attribute.text === "[") depth += 1;
        if (attribute.text === "]" && --depth === 0) break;
        if (attribute.literal || attribute.text !== "path" || tokens[cursor + 1]?.text !== "=")
          continue;
        const value = tokens[cursor + 2];
        if (!value?.literal || !/^"[^"\\]*"$/u.test(value.text)) {
          violations.push("Module path requires one explicit ordinary path literal");
          continue;
        }
        const target = resolve(root, dirname(path), value.text.slice(1, -1));
        if (!target.startsWith(`${resolve(root, owner)}/`))
          violations.push(`Module path escapes owner ${owner}`);
      }
    }
    if (tokens[index + 1]?.text !== "!") continue;
    if (token.text === "include")
      violations.push("Rust source inclusion bypasses package ownership");
    if (!["include_str", "include_bytes"].includes(token.text)) continue;
    const argument = tokens[index + 3];
    const closing = tokens[index + 4]?.text;
    if (
      !argument?.literal ||
      !/^"[^"\\]*"$/u.test(argument.text) ||
      !["}", ")", "]"].includes(closing ?? "")
    ) {
      violations.push("Resource inclusion requires one explicit ordinary path literal");
      continue;
    }
    const target = resolve(root, dirname(path), argument.text.slice(1, -1));
    const isApplicationLicense =
      owner === "apps/desktop" &&
      token.text === "include_str" &&
      ["LICENSE", "NOTICE"].some((document) => target === resolve(root, document));
    const applicationAssets: Record<string, readonly string[]> = {
      "apps/desktop/src-tauri/src/agent_icons.rs": [
        "packages/ui/src/assets/agent-icons.json",
        "packages/ui/tests/fixtures/agent-icons.json",
      ],
      "apps/desktop/src-tauri/src/agent.rs": [
        "packages/adapter-mcp-apps-view/src/assets/rich-html-app.html",
      ],
      "apps/desktop/src-tauri/src/mcp_apps.rs": [
        "packages/adapter-mcp-apps-view/src/assets/rich-html-app.html",
        "packages/adapter-mcp-apps-view/src/assets/rich-html-links.js",
        "packages/adapter-mcp-apps-host/src/assets/sandbox-proxy.html",
        "packages/adapter-mcp-apps-host/src/assets/sandbox-proxy.js",
      ],
    };
    const isApplicationAsset =
      owner === "apps/desktop" &&
      applicationAssets[path]?.some((asset) => target === resolve(root, asset)) &&
      (token.text === "include_str" ||
        (token.text === "include_bytes" &&
          target ===
            resolve(root, "packages/adapter-mcp-apps-view/src/assets/rich-html-links.js")));
    if (
      !target.startsWith(`${resolve(root, owner)}/`) &&
      !isApplicationLicense &&
      !isApplicationAsset
    )
      violations.push(`Resource escapes owner ${owner}`);
  }
  return violations;
}
