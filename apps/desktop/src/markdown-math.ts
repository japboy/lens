import katex from "katex";
import { Lexer, type Marked, type Token, type TokensList } from "marked";

export interface MathSpan {
  raw: string;
  tex: string;
  displayMode: boolean;
}

export const MATH_LIMITS = { count: 128, characters: 8192, totalCharacters: 65536 } as const;

function escaped(source: string, index: number): boolean {
  let slashes = 0;
  while (index > 0 && source[--index] === "\\") slashes++;
  return slashes % 2 === 1;
}

/** Reads only a complete expression at the beginning of a Markdown token. */
export function readMathSpan(source: string): MathSpan | undefined {
  const opening = source.slice(0, 2);
  const closing =
    opening === "\\(" ? "\\)" : opening === "\\[" ? "\\]" : opening === "$$" ? "$$" : undefined;
  if (!closing) return undefined;
  let end = source.indexOf(closing, 2);
  while (end >= 0 && escaped(source, end)) end = source.indexOf(closing, end + 2);
  if (end < 0) return undefined;
  const tex = source.slice(2, end);
  if (opening === "\\(" && /[\r\n]/u.test(tex)) return undefined;
  return { raw: source.slice(0, end + 2), tex, displayMode: opening !== "\\(" };
}

function maskMath(masked: string, source: string): string {
  const result = masked.split("");
  const opening = /\\[([]|\$\$/gu;
  let match: RegExpExecArray | null;
  while ((match = opening.exec(source))) {
    if (
      escaped(source, match.index) ||
      !["++", "$$"].includes(masked.slice(match.index, match.index + 2))
    )
      continue;
    const math = readMathSpan(source.slice(match.index));
    const remainder = source.slice(match.index);
    const length =
      math?.raw.length ??
      (remainder.startsWith("\\(") ? remainder.split(/\r?\n/u)[0].length : remainder.length);
    result.fill("a", match.index, match.index + length);
    opening.lastIndex = match.index + length;
  }
  return result.join("");
}

function literalHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

export function installMath(
  parser: Marked,
  render: (math: MathSpan) => string,
): (source: string) => TokensList {
  let inlineSource = "";
  parser.use({
    hooks: { emStrongMask: (masked) => maskMath(masked, inlineSource) },
    extensions: [
      {
        name: "lensMathBlock",
        level: "block",
        start: (source) => source.search(/(?:^|\n)(?:\\\[|\$\$)/u),
        tokenizer(source) {
          if (!source.startsWith("\\[") && !source.startsWith("$$")) return undefined;
          const math = readMathSpan(source);
          if (math) return { type: "lensMathBlock", ...math };
          return { type: "lensMathBlock", raw: source, literal: true };
        },
        renderer(token) {
          return `<p>${token.literal ? literalHtml(token.raw) : render(token as Token & MathSpan)}</p>\n`;
        },
      },
      {
        name: "lensMath",
        level: "inline",
        start: (source) => source.search(/\\[([]|\$\$/u),
        tokenizer(source) {
          if (this.lexer.state.inLink || this.lexer.state.inRawBlock) return undefined;
          const math = readMathSpan(source);
          if (math) return { type: "lensMath", ...math };
          if (/^(?:\\[([]|\$\$)/u.test(source)) {
            const raw = source.startsWith("\\(") ? source.split(/\r?\n/u)[0] : source;
            return { type: "lensMath", raw, literal: true };
          }
          return undefined;
        },
        renderer(token) {
          return token.literal ? literalHtml(token.raw) : render(token as Token & MathSpan);
        },
      },
    ],
  });
  // Marked masks escaped punctuation before emStrongMask. Capture the corresponding
  // original source so backslash delimiters retain their exact character positions.
  class MathLexer extends Lexer {
    override inlineTokens(source: string, tokens?: Token[]): Token[] {
      inlineSource = source;
      return super.inlineTokens(source, tokens);
    }
  }
  return (source) => new MathLexer(parser.defaults).lex(source);
}

const VOID_TAGS = new Set([
  "area",
  "base",
  "br",
  "col",
  "embed",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
]);

/** Inline raw HTML owns its contents; math never interprets those contents. */
export function excludeHtmlMath(tokens: TokensList | Token[], inheritedDepth = 0): void {
  const openTags: string[] = [];
  for (const token of tokens) {
    if (token.type === "html") {
      // Each inline HTML token is one tag, comment, or declaration. Inspect only
      // its leading tag name: tag-looking text in comments/attributes is inert.
      const tag = token.block ? null : /^<(\/?)([a-z][\w:-]*)\b/iu.exec(token.raw);
      if (tag && !VOID_TAGS.has(tag[2].toLowerCase())) {
        const name = tag[2].toLowerCase();
        if (tag[1]) {
          const index = openTags.lastIndexOf(name);
          if (index >= 0) openTags.splice(index);
        } else {
          // HTML ignores a self-closing slash on nonvoid elements.
          openTags.push(name);
        }
      }
    } else if (
      (token.type === "lensMath" || token.type === "lensMathBlock") &&
      inheritedDepth + openTags.length > 0
    ) {
      token.literal = true;
    }
    if ("tokens" in token && Array.isArray(token.tokens))
      excludeHtmlMath(token.tokens, inheritedDepth + openTags.length);
    if (token.type === "list") for (const item of token.items) excludeHtmlMath(item.tokens);
    if (token.type === "table") {
      for (const cell of token.header) excludeHtmlMath(cell.tokens);
      for (const row of token.rows) for (const cell of row) excludeHtmlMath(cell.tokens);
    }
  }
}

export function mathElement(math: MathSpan, permitted: boolean): HTMLElement {
  const element = document.createElement("span");
  element.className = math.displayMode ? "lens-math lens-math-display" : "lens-math";
  if (permitted) {
    try {
      katex.render(math.tex, element, {
        displayMode: math.displayMode,
        output: "htmlAndMathml",
        trust: false,
        throwOnError: true,
        strict: "error",
        maxExpand: 1000,
        maxSize: 20,
        macros: {},
      });
      return element;
    } catch {
      // Malformed, unsupported, or over-budget expressions remain readable text.
    }
  }
  element.classList.add("lens-math-fallback");
  element.textContent = math.raw;
  return element;
}
