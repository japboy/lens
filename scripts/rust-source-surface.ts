import { rustTokens } from "./rust-source-boundaries.ts";

type Token = ReturnType<typeof rustTokens>[number];
const CLOSING = new Map([
  ["(", ")"],
  ["[", "]"],
  ["{", "}"],
]);

export function groupEnd(tokens: readonly Token[], start: number): number {
  const expected = CLOSING.get(tokens[start]?.text ?? "");
  if (!expected) throw new Error("Expected a Rust token group");
  for (let index = start + 1; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if (token.literal) continue;
    if (token.text === expected) return index;
    if (CLOSING.has(token.text)) index = groupEnd(tokens, index);
    else if ([")", "]", "}"].includes(token.text)) throw new Error("Mismatched Rust token group");
  }
  throw new Error("Incomplete Rust token group");
}

function statementEnd(tokens: readonly Token[], start: number): number {
  for (let index = start; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if (token.literal) continue;
    if (token.text === ";") return index;
    if (CLOSING.has(token.text)) {
      const end = groupEnd(tokens, index);
      if (token.text === "{") return end;
      index = end;
    }
  }
  throw new Error("Incomplete Rust item or statement");
}

// Deliberately narrower than the Rust grammar: keep every non-body token, recurse
// only into impl/trait/module item lists, and reject unknown item forms. Rustc is the
// syntax/type authority. This comparator grants no permission for unparsed syntax.
export function rustDeclarationSurface(source: string): string {
  const tokens = rustTokens(source);
  function items(start: number, end: number): string[] {
    const result: string[] = [];
    let index = start;
    while (index < end) {
      if (tokens[index]!.text === ";") {
        result.push(";");
        index += 1;
        continue;
      }
      const itemStart = index;
      let testOnly = false;
      while (tokens[index]?.text === "#") {
        const inner = tokens[index + 1]?.text === "!";
        const open = index + (inner ? 2 : 1);
        if (tokens[open]?.text !== "[") throw new Error("Unsupported Rust attribute");
        const close = groupEnd(tokens, open);
        if (
          tokens
            .slice(open + 1, close)
            .map((token) => token.text)
            .join("") === "cfg(test)"
        )
          testOnly = true;
        index = close + 1;
        if (inner) {
          result.push(...tokens.slice(itemStart, index).map((token) => token.text));
          break;
        }
      }
      if (tokens[itemStart]?.text === "#" && tokens[itemStart + 1]?.text === "!") continue;
      if (index >= end) throw new Error("Attribute without a Rust item");
      let keyword = index;
      if (tokens[keyword]?.text === "pub") {
        keyword += 1;
        if (tokens[keyword]?.text === "(") keyword = groupEnd(tokens, keyword) + 1;
      }
      if (tokens[keyword]?.text === "async") keyword += 1;
      if (tokens[keyword]?.text === "const" && tokens[keyword + 1]?.text === "fn") keyword += 1;
      const kind = tokens[keyword]?.text;
      if (
        ![
          "fn",
          "impl",
          "trait",
          "mod",
          "struct",
          "enum",
          "use",
          "type",
          "const",
          "static",
        ].includes(kind ?? "")
      )
        throw new Error(`Unreviewed Rust item: ${kind}`);
      const last = statementEnd(tokens, index);
      if (last >= end) throw new Error("Rust item escapes its containing scope");
      if (!testOnly) {
        let body = -1;
        for (let cursor = keyword + 1; cursor <= last; cursor += 1) {
          if (tokens[cursor]!.literal) continue;
          if (tokens[cursor]!.text === "{") {
            body = cursor;
            break;
          }
          if (CLOSING.has(tokens[cursor]!.text)) cursor = groupEnd(tokens, cursor);
        }
        if (body !== -1 && ["fn", "impl", "trait", "mod"].includes(kind!)) {
          result.push(...tokens.slice(itemStart, body + 1).map((token) => token.text));
          // A const function body can determine a consumer's type/array length.
          // Keep it in the declaration surface, even when the signature is stable.
          if (kind === "fn" && tokens[keyword - 1]?.text === "const")
            result.push(...tokens.slice(body + 1, last).map((token) => token.text));
          else if (kind === "fn") result.push("<implementation>");
          else result.push(...items(body + 1, last));
          result.push("}");
        } else result.push(...tokens.slice(itemStart, last + 1).map((token) => token.text));
      }
      index = last + 1;
    }
    return result;
  }
  return JSON.stringify(items(0, tokens.length));
}

const COMMON_CONDITIONS = new Set(["test", "debug_assertions", "all(test,debug_assertions)"]);
const NATIVE_ENTRY_STATEMENTS: Record<string, readonly string[]> = {
  "apps/desktop/src-tauri/src/lib.rs": [
    '#[cfg(target_os = "macos")] mod native;',
    '#[cfg(target_os = "macos")] pub use native::run;',
    '#[cfg(target_os = "macos")] native::configure_activation(app, validate_a11y);',
  ],
  "apps/desktop/src-tauri/src/main.rs": [
    '#[cfg(target_os = "macos")] fn main() { lens_lib::run(); }',
    '#[cfg(not(target_os = "macos"))] fn main() { eprintln!("Lens currently supports macOS only; this target is for common-library verification."); std::process::exit(1); }',
  ],
};
const tokensOf = (source: string) => JSON.stringify(rustTokens(source).map((token) => token.text));

export function commonShellConditionalViolations(path: string, source: string): string[] {
  const tokens = rustTokens(source);
  const violations: string[] = [];
  for (const [index, token] of tokens.entries()) {
    if (token.literal) continue;
    if (token.text === "cfg_attr")
      violations.push("Common shell cannot conditionally rewrite attributes");
    if (token.text !== "cfg") continue;
    const macro = tokens[index + 1]?.text === "!";
    const open = index + (macro ? 2 : 1);
    const close = groupEnd(tokens, open);
    // cfg! produces a bool; unlike an attribute it does not remove consumer code
    // from type checking. Common host effects already require native verification.
    if (macro) continue;
    const condition = tokens
      .slice(open + 1, close)
      .map((entry) => entry.text)
      .join("");
    if (COMMON_CONDITIONS.has(condition)) continue;
    if (tokens[index - 1]?.text !== "[" || tokens[index - 2]?.text !== "#") {
      violations.push("Unreviewed common-shell conditional compilation");
      continue;
    }
    const end = statementEnd(tokens, close + 2);
    const actual = JSON.stringify(tokens.slice(index - 2, end + 1).map((entry) => entry.text));
    if (!(NATIVE_ENTRY_STATEMENTS[path] ?? []).some((statement) => tokensOf(statement) === actual))
      violations.push("Native-only consumer outside the finite composition entry points");
  }
  return violations;
}

const PLATFORM_TYPES = new Set([
  "Services",
  "Presentation",
  "WindowPresentation",
  "PresentationFuture",
]);
export function nativeCompositionViolations(source: string): string[] {
  const tokens = rustTokens(source);
  const violations: string[] = [];
  for (const [index, token] of tokens.entries()) {
    if (token.literal) continue;
    if (["usecase", "domain", "lens_lib", "include", "macro_rules", "path"].includes(token.text))
      violations.push(`Native composition cannot access ${token.text}`);
    if (token.text === "#") {
      if (tokens[index + 1]?.text !== "[")
        violations.push("Unreviewed native composition attribute");
      else {
        const end = groupEnd(tokens, index + 1);
        const actual = JSON.stringify(tokens.slice(index, end + 1).map((entry) => entry.text));
        if (actual !== tokensOf("#[cfg_attr(mobile, tauri::mobile_entry_point)]"))
          violations.push("Unreviewed native composition attribute");
      }
    }
    if (
      /^[A-Za-z_]/u.test(token.text) &&
      tokens[index + 1]?.text === "!" &&
      ["(", "[", "{"].includes(tokens[index + 2]?.text ?? "") &&
      token.text !== "format"
    )
      violations.push("Unreviewed native composition macro");
    if (!["crate", "super"].includes(token.text)) continue;
    if (tokens[index - 1]?.text === "(" && tokens[index - 2]?.text === "pub") continue;
    const at = (distance: number) => tokens[index + distance]?.text;
    if (at(1) === "::" && at(2) === "run_with_runtime" && at(3) !== "::") continue;
    if (at(1) === "::" && at(2) === "platform" && at(3) === "::") {
      if (PLATFORM_TYPES.has(at(4) ?? "") && at(5) !== "::") continue;
      if (at(4) === "{") {
        const close = groupEnd(tokens, index + 4);
        if (
          tokens
            .slice(index + 5, close)
            .every((entry) => entry.text === "," || PLATFORM_TYPES.has(entry.text))
        )
          continue;
      }
    }
    violations.push(
      "Native composition may only consume the common runtime entry and explicit platform effect types",
    );
  }
  return violations;
}
