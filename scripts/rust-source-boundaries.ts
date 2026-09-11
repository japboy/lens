import { dirname, resolve } from "node:path";

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
  // Iteration order is randomised per process, so a portable crate that serialized or
  // displayed one of these would produce a different projection digest on every run. The
  // ordered collections are already used throughout; this keeps that from regressing
  // silently into a flaky digest rather than a failed check.
  "HashMap",
  "HashSet",
  "hash_map",
  "hash_set",
  "RandomState",
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
    if (!target.startsWith(`${resolve(root, owner)}/`) && !isApplicationLicense)
      violations.push(`Resource escapes owner ${owner}`);
  }
  return violations;
}
